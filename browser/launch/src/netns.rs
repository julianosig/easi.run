//! Chrome's own network namespace. The browser container has no network, so its only link is loopback and Chrome
//! reads "offline" (`navigator.onLine` false, no network quality estimate), which no ordinary desktop shows. The
//! launcher therefore moves itself, before it starts any thread, into a new user and network namespace that has an
//! online link: a veth pair whose two ends only reach each other, with an address on one of them. Nothing leaves it;
//! Chrome still reaches only the proxy, through the launcher's loopback forwarder, which then runs in this namespace.
//!
//! The user namespace maps only the launcher's own uid and gid, unchanged, so every peer-credential check outside it
//! (the display, the proxy, the buses) still sees uid 10001. The launcher drops every capability it holds in the new
//! namespace once the link is up, and Chrome, a non-root uid there, never has one.

use std::os::fd::{AsFd, OwnedFd};

use rustix::net::netlink::SocketAddrNetlink;
use rustix::net::{AddressFamily, RecvFlags, SendFlags, SocketType};
use rustix::thread::{CapabilitySet, CapabilitySets, UnshareFlags};

/// The interfaces the namespace gets: loopback is always index 1; the veth pair's ends are named, and their indexes
/// are the kernel's choice, read back by name: a host with tunnel modules loaded puts fallback devices (`tunl0`,
/// `sit0`, ...) in every new namespace, so no index past loopback is free for certain.
const LOOPBACK: i32 = 1;
const LINK: &str = "eth0";
const PEER: &str = "eth1";
/// The link's address, as on a home network.
const ADDRESS: [u8; 4] = [192, 168, 1, 100];
const PREFIX: u8 = 24;

const RTM_NEWLINK: u16 = 16;
const RTM_NEWADDR: u16 = 20;
const NLMSG_ERROR: u16 = 2;
const NLM_F_REQUEST: u16 = 0x1;
const NLM_F_ACK: u16 = 0x4;
const NLM_F_EXCL: u16 = 0x200;
const NLM_F_CREATE: u16 = 0x400;
const IFLA_IFNAME: u16 = 3;
const IFLA_LINKINFO: u16 = 18;
const IFLA_INFO_KIND: u16 = 1;
const IFLA_INFO_DATA: u16 = 2;
const VETH_INFO_PEER: u16 = 1;
const IFLA_AF_SPEC: u16 = 26;
const IFLA_INET6_ADDR_GEN_MODE: u16 = 8;
const IN6_ADDR_GEN_MODE_NONE: u8 = 1;
const AF_INET6: u16 = 10;
const IFA_ADDRESS: u16 = 1;
const IFA_LOCAL: u16 = 2;
const IFF_UP: u32 = 0x1;
const AF_INET: u8 = 2;

/// Moves the launcher into its own user and network namespace with an online link, then drops its capabilities. It
/// must run before the launcher starts any thread (the kernel refuses a new user namespace to a threaded process).
pub fn isolate() -> Result<(), String> {
    let (uid, gid) = (rustix::process::getuid().as_raw(), rustix::process::getgid().as_raw());
    // The safe call is deprecated only for `UnshareFlags::FILES`, which this does not unshare.
    #[allow(deprecated)]
    rustix::thread::unshare(UnshareFlags::NEWUSER | UnshareFlags::NEWNET).map_err(|e| format!("unshare: {e}"))?;
    let write = |path: &str, text: String| std::fs::write(path, text).map_err(|e| format!("{path}: {e}"));
    write("/proc/self/setgroups", "deny".to_owned())?;
    write("/proc/self/uid_map", format!("{uid} {uid} 1"))?;
    write("/proc/self/gid_map", format!("{gid} {gid} 1"))?;
    let socket =
        rustix::net::socket(AddressFamily::NETLINK, SocketType::RAW, None).map_err(|e| format!("netlink: {e}"))?;
    let mut seq = 0;
    let mut request = |message: Vec<u8>| {
        seq += 1;
        exchange(&socket, seq, message)
    };
    request(link_up(LOOPBACK)).map_err(|e| format!("loopback up: {e}"))?;
    request(new_veth()).map_err(|e| format!("veth pair: {e}"))?;
    let index = |name| match rustix::net::netdevice::name_to_index(&socket, name) {
        Ok(index) => i32::try_from(index).map_err(|_| format!("{name}: index {index}")),
        Err(e) => Err(format!("{name}: {e}")),
    };
    let (link, peer) = (index(LINK)?, index(PEER)?);
    for (name, index) in [(LINK, link), (PEER, peer)] {
        // A kernel booted with `ipv6.disable=1` has no IPv6 to turn off and refuses the attribute outright.
        match request(no_link_local(index)) {
            Err(e) if e == rustix::io::Errno::AFNOSUPPORT.to_string() => {}
            other => other.map_err(|e| format!("{name} without IPv6: {e}"))?,
        }
        request(link_up(index)).map_err(|e| format!("{name} up: {e}"))?;
    }
    request(new_address(link)).map_err(|e| format!("{LINK} address: {e}"))?;
    let none = CapabilitySet::empty();
    rustix::thread::set_capabilities(None, CapabilitySets { effective: none, permitted: none, inheritable: none })
        .map_err(|e| format!("dropping capabilities: {e}"))
}

/// Sends one request numbered `seq` and reads the kernel's acknowledgement.
fn exchange(socket: &OwnedFd, seq: u32, mut message: Vec<u8>) -> Result<(), String> {
    message[8..12].copy_from_slice(&seq.to_ne_bytes());
    let kernel = SocketAddrNetlink::new(0, 0);
    rustix::net::sendto(socket.as_fd(), &message, SendFlags::empty(), &kernel).map_err(|e| e.to_string())?;
    let mut reply = [0u8; 4096];
    let (n, _) = rustix::net::recv(socket.as_fd(), &mut reply, RecvFlags::empty()).map_err(|e| e.to_string())?;
    acknowledged(&reply[..n.min(reply.len())], seq)
}

/// The error code of the kernel's `NLMSG_ERROR` answer to `seq`: zero is success.
fn acknowledged(reply: &[u8], seq: u32) -> Result<(), String> {
    let field = |at: usize| reply.get(at..at + 4).map(|b| u32::from_ne_bytes(b.try_into().unwrap()));
    let kind = reply.get(4..6).map(|b| u16::from_ne_bytes(b.try_into().unwrap()));
    match (kind, field(8), field(16)) {
        (Some(NLMSG_ERROR), Some(s), Some(code)) if s == seq => match code as i32 {
            0 => Ok(()),
            errno => Err(rustix::io::Errno::from_raw_os_error(-errno).to_string()),
        },
        _ => Err("no acknowledgement".to_owned()),
    }
}

/// A netlink message: the header (length and sequence filled in later) and the body.
fn message(kind: u16, flags: u16, body: &[u8]) -> Vec<u8> {
    let mut m = Vec::with_capacity(16 + body.len());
    m.extend_from_slice(&((16 + body.len()) as u32).to_ne_bytes());
    m.extend_from_slice(&kind.to_ne_bytes());
    m.extend_from_slice(&(flags | NLM_F_REQUEST | NLM_F_ACK).to_ne_bytes());
    m.extend_from_slice(&[0; 8]);
    m.extend_from_slice(body);
    m
}

/// An attribute, padded to four bytes.
fn attribute(kind: u16, data: &[u8]) -> Vec<u8> {
    let mut a = Vec::with_capacity(4 + data.len() + 3);
    a.extend_from_slice(&((4 + data.len()) as u16).to_ne_bytes());
    a.extend_from_slice(&kind.to_ne_bytes());
    a.extend_from_slice(data);
    a.resize(a.len().next_multiple_of(4), 0);
    a
}

/// `ifinfomsg` for interface `index`, set up or left as it is.
fn interface(index: i32, up: bool) -> Vec<u8> {
    let flags = if up { IFF_UP } else { 0 };
    let mut i = vec![0u8; 4];
    i.extend_from_slice(&index.to_ne_bytes());
    i.extend_from_slice(&flags.to_ne_bytes());
    i.extend_from_slice(&flags.to_ne_bytes());
    i
}

fn name(text: &str) -> Vec<u8> {
    let mut n = text.as_bytes().to_vec();
    n.push(0);
    n
}

fn link_up(index: i32) -> Vec<u8> {
    message(RTM_NEWLINK, 0, &interface(index, true))
}

/// Before the link is up: no IPv6 link-local address, so the namespace's only addresses are its IPv4 one and loopback.
fn no_link_local(index: i32) -> Vec<u8> {
    let inet6 = attribute(AF_INET6, &attribute(IFLA_INET6_ADDR_GEN_MODE, &[IN6_ADDR_GEN_MODE_NONE]));
    let body = [interface(index, false), attribute(IFLA_AF_SPEC, &inet6)].concat();
    message(RTM_NEWLINK, 0, &body)
}

/// The veth pair, each end at its fixed name and at whatever index the kernel picks (zero asks it to), both down.
fn new_veth() -> Vec<u8> {
    let peer = [interface(0, false), attribute(IFLA_IFNAME, &name(PEER))].concat();
    let data = attribute(VETH_INFO_PEER, &peer);
    let info = [attribute(IFLA_INFO_KIND, b"veth"), attribute(IFLA_INFO_DATA, &data)].concat();
    let link = [interface(0, false), attribute(IFLA_IFNAME, &name(LINK))].concat();
    message(RTM_NEWLINK, NLM_F_CREATE | NLM_F_EXCL, &[link, attribute(IFLA_LINKINFO, &info)].concat())
}

fn new_address(index: i32) -> Vec<u8> {
    let mut body = vec![AF_INET, PREFIX, 0, 0];
    body.extend_from_slice(&index.to_ne_bytes());
    body.extend(attribute(IFA_LOCAL, &ADDRESS));
    body.extend(attribute(IFA_ADDRESS, &ADDRESS));
    message(RTM_NEWADDR, NLM_F_CREATE | NLM_F_EXCL, &body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_are_well_formed_netlink() {
        for m in [link_up(LOOPBACK), new_veth(), no_link_local(5), new_address(5)] {
            assert_eq!(u32::from_ne_bytes(m[0..4].try_into().unwrap()) as usize, m.len());
            assert_eq!(m.len() % 4, 0);
        }
        let veth = new_veth();
        assert!(veth.windows(5).any(|w| w == b"eth0\0") && veth.windows(5).any(|w| w == b"eth1\0"));
        assert!(veth.windows(4).any(|w| w == b"veth"));
        assert_eq!(&new_address(5)[16..20], &[AF_INET, PREFIX, 0, 0]);
        assert_eq!(&new_address(5)[20..24], &5u32.to_ne_bytes());
        // Neither end asks for an index: one taken by a fallback tunnel device would fail the whole namespace.
        assert_eq!(&veth[20..24], &0i32.to_ne_bytes());
        let peer = veth.windows(5).position(|w| w == b"eth1\0").unwrap() - 4 - 16;
        assert_eq!(&veth[peer + 4..peer + 8], &0i32.to_ne_bytes());
    }

    #[test]
    fn only_a_zero_error_for_the_same_request_is_success() {
        let reply = |seq: u32, code: i32| {
            let mut r = vec![0u8; 36];
            r[4..6].copy_from_slice(&NLMSG_ERROR.to_ne_bytes());
            r[8..12].copy_from_slice(&seq.to_ne_bytes());
            r[16..20].copy_from_slice(&code.to_ne_bytes());
            r
        };
        assert!(acknowledged(&reply(3, 0), 3).is_ok());
        assert!(acknowledged(&reply(3, -17), 3).is_err());
        assert!(acknowledged(&reply(2, 0), 3).is_err());
        assert!(acknowledged(&[], 3).is_err());
    }
}
