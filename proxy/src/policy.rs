//! The destination policy: which resolved addresses and ports the proxy may connect to.
//!
//! The policy is a closed deny list of every non-public range named by the design. An address embedding an IPv4
//! address (IPv4-mapped, NAT64, 6to4) is judged by the embedded address, so a public prefix cannot launder a private
//! destination. Production code contains no exception; the `bench` fixture is applied by the caller by exact name.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// The only destination ports the proxy connects to.
pub const ALLOWED_PORTS: [u16; 2] = [80, 443];

/// Why a destination is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Forbidden(pub &'static str);

const V4_DENY: &[(u32, u8, &str)] = &[
    (0x0000_0000, 8, "ipv4_this_network"),
    (0x0a00_0000, 8, "ipv4_private"),
    (0x6440_0000, 10, "ipv4_shared_cgnat"),
    (0x7f00_0000, 8, "ipv4_loopback"),
    (0xa9fe_0000, 16, "ipv4_link_local"),
    (0xac10_0000, 12, "ipv4_private"),
    (0xc000_0000, 24, "ipv4_ietf_protocol"),
    (0xc000_0200, 24, "ipv4_documentation"),
    (0xc058_6300, 24, "ipv4_6to4_relay"),
    (0xc0a8_0000, 16, "ipv4_private"),
    (0xc612_0000, 15, "ipv4_benchmarking"),
    (0xc633_6400, 24, "ipv4_documentation"),
    (0xcb00_7100, 24, "ipv4_documentation"),
    (0xe000_0000, 4, "ipv4_multicast"),
    (0xf000_0000, 4, "ipv4_reserved"),
];

/// How an IPv6 range is judged once an address falls in it.
#[derive(Clone, Copy)]
enum V6Rule {
    /// Always refused.
    Deny(&'static str),
    /// Judged by the IPv4 address in the given bit offset (from the most significant bit).
    Embedded(&'static str, u32),
}

const V6_DENY: &[(u128, u8, V6Rule)] = &[
    (0, 128, V6Rule::Deny("ipv6_unspecified")),
    (1, 128, V6Rule::Deny("ipv6_loopback")),
    (0xffff_0000_0000, 96, V6Rule::Embedded("ipv6_mapped", 96)),
    (0x0064_ff9b_u128 << 96, 96, V6Rule::Embedded("ipv6_nat64", 96)),
    (0x0064_ff9b_0001_u128 << 80, 48, V6Rule::Embedded("ipv6_nat64_local", 96)),
    (0x0100_u128 << 112, 64, V6Rule::Deny("ipv6_discard")),
    (0x2001_0db8_u128 << 96, 32, V6Rule::Deny("ipv6_documentation")),
    (0x2001_u128 << 112, 23, V6Rule::Deny("ipv6_ietf_protocol")),
    (0x2002_u128 << 112, 16, V6Rule::Embedded("ipv6_6to4", 16)),
    (0xfc00_u128 << 112, 7, V6Rule::Deny("ipv6_unique_local")),
    (0xfe80_u128 << 112, 10, V6Rule::Deny("ipv6_link_local")),
    (0xfec0_u128 << 112, 10, V6Rule::Deny("ipv6_site_local")),
    (0xff00_u128 << 112, 8, V6Rule::Deny("ipv6_multicast")),
];

/// Classifies one resolved address: `Ok` when the policy admits it, `Err` with the reason otherwise.
pub fn classify(addr: IpAddr) -> Result<(), Forbidden> {
    match addr {
        IpAddr::V4(v4) => classify_v4(v4),
        IpAddr::V6(v6) => classify_v6(v6),
    }
}

/// Classifies a destination port.
pub fn port_allowed(port: u16) -> bool {
    ALLOWED_PORTS.contains(&port)
}

fn classify_v4(addr: Ipv4Addr) -> Result<(), Forbidden> {
    let bits = u32::from(addr);
    match V4_DENY.iter().find(|(net, len, _)| prefix_match_u32(bits, *net, *len)) {
        Some((_, _, reason)) => Err(Forbidden(reason)),
        None => Ok(()),
    }
}

fn classify_v6(addr: Ipv6Addr) -> Result<(), Forbidden> {
    let bits = u128::from(addr);
    let Some((_, _, rule)) = V6_DENY.iter().find(|(net, len, _)| prefix_match_u128(bits, *net, *len)) else {
        return Ok(());
    };
    match *rule {
        V6Rule::Deny(reason) => Err(Forbidden(reason)),
        V6Rule::Embedded(reason, offset) => {
            let embedded = Ipv4Addr::from((bits >> (96 - offset)) as u32);
            classify_v4(embedded).map_err(|_| Forbidden(reason))
        }
    }
}

/// The address to dial for a vetted address: an IPv4-mapped IPv6 address is dialed as the IPv4 address it names.
pub fn dial_address(addr: IpAddr) -> IpAddr {
    match addr {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(addr, IpAddr::V4),
        IpAddr::V4(_) => addr,
    }
}

fn prefix_match_u32(bits: u32, net: u32, len: u8) -> bool {
    let mask = if len == 0 { 0 } else { u32::MAX << (32 - u32::from(len)) };
    bits & mask == net & mask
}

fn prefix_match_u128(bits: u128, net: u128, len: u8) -> bool {
    let mask = if len == 0 { 0 } else { u128::MAX << (128 - u32::from(len)) };
    bits & mask == net & mask
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every row is an address and the expected verdict: `None` admits it, `Some(reason)` refuses it.
    const TABLE: &[(&str, Option<&str>)] = &[
        // 0.0.0.0/8
        ("0.0.0.0", Some("ipv4_this_network")),
        ("0.255.255.255", Some("ipv4_this_network")),
        ("1.0.0.0", None),
        // 10.0.0.0/8
        ("9.255.255.255", None),
        ("10.0.0.0", Some("ipv4_private")),
        ("10.255.255.255", Some("ipv4_private")),
        ("11.0.0.0", None),
        // 100.64.0.0/10
        ("100.63.255.255", None),
        ("100.64.0.0", Some("ipv4_shared_cgnat")),
        ("100.127.255.255", Some("ipv4_shared_cgnat")),
        ("100.128.0.0", None),
        // 127.0.0.0/8
        ("126.255.255.255", None),
        ("127.0.0.0", Some("ipv4_loopback")),
        ("127.0.0.1", Some("ipv4_loopback")),
        ("127.255.255.255", Some("ipv4_loopback")),
        ("128.0.0.0", None),
        // 169.254.0.0/16
        ("169.253.255.255", None),
        ("169.254.0.0", Some("ipv4_link_local")),
        ("169.254.169.254", Some("ipv4_link_local")),
        ("169.254.255.255", Some("ipv4_link_local")),
        ("169.255.0.0", None),
        // 172.16.0.0/12
        ("172.15.255.255", None),
        ("172.16.0.0", Some("ipv4_private")),
        ("172.31.255.255", Some("ipv4_private")),
        ("172.32.0.0", None),
        // 192.0.0.0/24 and 192.0.2.0/24
        ("191.255.255.255", None),
        ("192.0.0.0", Some("ipv4_ietf_protocol")),
        ("192.0.0.255", Some("ipv4_ietf_protocol")),
        ("192.0.1.0", None),
        ("192.0.1.255", None),
        ("192.0.2.0", Some("ipv4_documentation")),
        ("192.0.2.255", Some("ipv4_documentation")),
        ("192.0.3.0", None),
        // 192.88.99.0/24
        ("192.88.98.255", None),
        ("192.88.99.0", Some("ipv4_6to4_relay")),
        ("192.88.99.255", Some("ipv4_6to4_relay")),
        ("192.88.100.0", None),
        // 192.168.0.0/16
        ("192.167.255.255", None),
        ("192.168.0.0", Some("ipv4_private")),
        ("192.168.255.255", Some("ipv4_private")),
        ("192.169.0.0", None),
        // 198.18.0.0/15
        ("198.17.255.255", None),
        ("198.18.0.0", Some("ipv4_benchmarking")),
        ("198.19.255.255", Some("ipv4_benchmarking")),
        ("198.20.0.0", None),
        // 198.51.100.0/24
        ("198.51.99.255", None),
        ("198.51.100.0", Some("ipv4_documentation")),
        ("198.51.100.255", Some("ipv4_documentation")),
        ("198.51.101.0", None),
        // 203.0.113.0/24
        ("203.0.112.255", None),
        ("203.0.113.0", Some("ipv4_documentation")),
        ("203.0.113.255", Some("ipv4_documentation")),
        ("203.0.114.0", None),
        // 224.0.0.0/4 and 240.0.0.0/4 (incl. broadcast)
        ("223.255.255.255", None),
        ("224.0.0.0", Some("ipv4_multicast")),
        ("239.255.255.255", Some("ipv4_multicast")),
        ("240.0.0.0", Some("ipv4_reserved")),
        ("255.255.255.255", Some("ipv4_reserved")),
        // Public IPv4 controls
        ("8.8.8.8", None),
        ("93.184.216.34", None),
        // ::/128 and ::1/128
        ("::", Some("ipv6_unspecified")),
        ("::1", Some("ipv6_loopback")),
        ("::2", None),
        // ::ffff:0:0/96 judged by the embedded IPv4 address
        ("::fffe:ffff:ffff", None),
        ("::ffff:0.0.0.0", Some("ipv6_mapped")),
        ("::ffff:127.0.0.1", Some("ipv6_mapped")),
        ("::ffff:10.0.0.1", Some("ipv6_mapped")),
        ("::ffff:169.254.169.254", Some("ipv6_mapped")),
        ("::ffff:255.255.255.255", Some("ipv6_mapped")),
        ("::ffff:8.8.8.8", None),
        ("::1:0:0:0", None),
        // 64:ff9b::/96 judged by the embedded IPv4 address
        ("64:ff9a:ffff:ffff:ffff:ffff:ffff:ffff", None),
        ("64:ff9b::", Some("ipv6_nat64")),
        ("64:ff9b::7f00:1", Some("ipv6_nat64")),
        ("64:ff9b::a00:1", Some("ipv6_nat64")),
        ("64:ff9b::c0a8:101", Some("ipv6_nat64")),
        ("64:ff9b::ffff:ffff", Some("ipv6_nat64")),
        ("64:ff9b::808:808", None),
        ("64:ff9b::1:0:0", None),
        // 64:ff9b:1::/48 judged by the embedded IPv4 address
        ("64:ff9b:0:ffff:ffff:ffff:ffff:ffff", None),
        ("64:ff9b:1::", Some("ipv6_nat64_local")),
        ("64:ff9b:1::7f00:1", Some("ipv6_nat64_local")),
        ("64:ff9b:1:ffff:ffff:ffff:a9fe:a9fe", Some("ipv6_nat64_local")),
        ("64:ff9b:1::808:808", None),
        ("64:ff9b:1:ffff:ffff:ffff:808:808", None),
        ("64:ff9b:2::", None),
        // 100::/64
        ("ff:ffff:ffff:ffff:ffff:ffff:ffff:ffff", None),
        ("100::", Some("ipv6_discard")),
        ("100::ffff:ffff:ffff:ffff", Some("ipv6_discard")),
        ("100:0:0:1::", None),
        // 2001::/23 (incl. Teredo 2001::/32) and 2001:db8::/32
        ("2000:ffff:ffff:ffff:ffff:ffff:ffff:ffff", None),
        ("2001::", Some("ipv6_ietf_protocol")),
        ("2001:0:ffff:ffff:ffff:ffff:ffff:ffff", Some("ipv6_ietf_protocol")),
        ("2001:1ff:ffff:ffff:ffff:ffff:ffff:ffff", Some("ipv6_ietf_protocol")),
        ("2001:200::", None),
        ("2001:db7:ffff:ffff:ffff:ffff:ffff:ffff", None),
        ("2001:db8::", Some("ipv6_documentation")),
        ("2001:db8:ffff:ffff:ffff:ffff:ffff:ffff", Some("ipv6_documentation")),
        ("2001:db9::", None),
        // 2002::/16 judged by the embedded IPv4 address
        ("2001:ffff:ffff:ffff:ffff:ffff:ffff:ffff", None),
        ("2002::", Some("ipv6_6to4")),
        ("2002:7f00:1::", Some("ipv6_6to4")),
        ("2002:c0a8:101:ffff:ffff:ffff:ffff:ffff", Some("ipv6_6to4")),
        ("2002:ffff:ffff::", Some("ipv6_6to4")),
        ("2002:808:808::1", None),
        ("2003::", None),
        // fc00::/7
        ("fbff:ffff:ffff:ffff:ffff:ffff:ffff:ffff", None),
        ("fc00::", Some("ipv6_unique_local")),
        ("fdff:ffff:ffff:ffff:ffff:ffff:ffff:ffff", Some("ipv6_unique_local")),
        ("fe00::", None),
        // fe80::/10 and fec0::/10
        ("fe7f:ffff:ffff:ffff:ffff:ffff:ffff:ffff", None),
        ("fe80::", Some("ipv6_link_local")),
        ("febf:ffff:ffff:ffff:ffff:ffff:ffff:ffff", Some("ipv6_link_local")),
        ("fec0::", Some("ipv6_site_local")),
        ("feff:ffff:ffff:ffff:ffff:ffff:ffff:ffff", Some("ipv6_site_local")),
        // ff00::/8
        ("ff00::", Some("ipv6_multicast")),
        ("ff02::1", Some("ipv6_multicast")),
        ("ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff", Some("ipv6_multicast")),
        // Public IPv6 controls
        ("2606:4700:4700::1111", None),
        ("2a00:1450:4001::200e", None),
    ];

    #[test]
    fn classifier_table() {
        for (text, expected) in TABLE {
            let addr: IpAddr = text.parse().unwrap_or_else(|_| panic!("bad table address {text}"));
            let verdict = classify(addr).err().map(|Forbidden(reason)| reason);
            assert_eq!(verdict, *expected, "address {text}");
        }
    }

    #[test]
    fn only_web_ports_are_allowed() {
        for port in [0, 1, 22, 25, 53, 79, 81, 442, 444, 3128, 8080, 8443, 65535] {
            assert!(!port_allowed(port), "port {port}");
        }
        assert!(port_allowed(80));
        assert!(port_allowed(443));
    }

    #[test]
    fn mapped_addresses_dial_as_ipv4() {
        let mapped: IpAddr = "::ffff:8.8.8.8".parse().unwrap();
        assert_eq!(dial_address(mapped), IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)));
        let native: IpAddr = "2606:4700:4700::1111".parse().unwrap();
        assert_eq!(dial_address(native), native);
    }
}
