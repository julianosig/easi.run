//! The proxy forwarder: the browser has no network (its own namespace's link reaches nothing), so Chromium speaks to
//! its proxy at a loopback address and each connection is carried unchanged to the proxy's Unix socket. It decides
//! nothing: the proxy vets every request. Connections are bounded to what the descriptor limit leaves room for, and
//! what comes from the network waits at the downloads' gate (see `gate`).

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use rustix::process::{Resource, getrlimit};

use crate::gate::Gate;

const MAX_CONNECTIONS: usize = 256;
/// What one connection holds: the client's socket and the proxy's.
const DESCRIPTORS_PER_CONNECTION: u64 = 2;
/// Descriptors left for the rest of the launcher: its files, downloads, uploads, Wayland, and D-Bus.
const RESERVED_DESCRIPTORS: u64 = 256;
/// The fewest connections Chromium needs: it opens up to 32 to one proxy.
const MIN_CONNECTIONS: usize = 32;
/// How long accepting pauses after it failed (out of descriptors, say), instead of failing again at once.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(50);

/// The connections the soft descriptor limit leaves room for, at most `MAX_CONNECTIONS`; `None` when that is fewer
/// than Chromium needs.
fn connection_cap(soft_limit: Option<u64>) -> Option<usize> {
    let room = soft_limit.map_or(u64::MAX, |n| n.saturating_sub(RESERVED_DESCRIPTORS) / DESCRIPTORS_PER_CONNECTION);
    let cap = usize::try_from(room).unwrap_or(usize::MAX).min(MAX_CONNECTIONS);
    (cap >= MIN_CONNECTIONS).then_some(cap)
}

fn copy(mut from: impl Read, mut to: impl Write, gate: Option<&Gate>, done: impl FnOnce()) {
    let mut buf = [0u8; 16 * 1024];
    loop {
        match from.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if let Some(gate) = gate {
                    gate.admit(n as u64);
                }
                if to.write_all(&buf[..n]).is_err() {
                    break;
                }
            }
        }
    }
    done();
}

/// Holds one of the bounded connection slots and gives it back when dropped, so a slot returns even when the
/// connection's threads never start or one of them unwinds; a lost slot would never come back.
struct Slot(Arc<AtomicUsize>);

impl Slot {
    fn take(live: &Arc<AtomicUsize>, max: usize) -> Option<Self> {
        if live.fetch_add(1, Ordering::Relaxed) >= max {
            live.fetch_sub(1, Ordering::Relaxed);
            return None;
        }
        Some(Self(live.clone()))
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Carries one connection, each direction on its own thread over the same two sockets.
fn serve(client: TcpStream, socket: &str, gate: &Gate) {
    // Each read from the proxy is written on at once: under Nagle, a write after a small one would wait for its ACK,
    // which Chromium's delayed ACK holds back up to 40 ms. The proxy's side is a Unix socket, which has no such delay.
    let _ = client.set_nodelay(true);
    let Ok(upstream) = UnixStream::connect(socket) else { return };
    std::thread::scope(|scope| {
        let up = std::thread::Builder::new().stack_size(64 * 1024).spawn_scoped(scope, || {
            copy(&client, &upstream, None, || {
                let _ = upstream.shutdown(Shutdown::Write);
            });
        });
        // Without the second thread the connection cannot be carried; dropping both ends after the scope closes it.
        if up.is_ok() {
            copy(&upstream, &client, Some(gate), || {
                let _ = client.shutdown(Shutdown::Write);
            });
        }
    });
}

/// Serves `listen` until the process ends, on its own thread.
pub fn start(listen: &str, socket: &str, gate: Arc<Gate>) -> std::io::Result<()> {
    let soft_limit = getrlimit(Resource::Nofile).current;
    let max = connection_cap(soft_limit).ok_or_else(|| {
        std::io::Error::other(format!("the descriptor limit {soft_limit:?} leaves too few for the proxy's connections"))
    })?;
    let listener = TcpListener::bind(listen)?;
    let socket: Arc<str> = socket.into();
    std::thread::Builder::new().name("proxy-forward".into()).spawn(move || {
        let live = Arc::new(AtomicUsize::new(0));
        for client in listener.incoming() {
            let Ok(client) = client else {
                std::thread::sleep(ACCEPT_BACKOFF);
                continue;
            };
            let Some(slot) = Slot::take(&live, max) else { continue };
            let (socket, gate) = (socket.clone(), gate.clone());
            // A thread that fails to start drops its closure, and with it the slot.
            let _ = std::thread::Builder::new().stack_size(64 * 1024).spawn(move || {
                let _slot = slot;
                serve(client, &socket, &gate);
            });
        }
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slot_returns_when_dropped_and_the_bound_holds() {
        let live = Arc::new(AtomicUsize::new(0));
        let slots: Vec<_> =
            (0..MAX_CONNECTIONS).map(|_| Slot::take(&live, MAX_CONNECTIONS).expect("a free slot")).collect();
        assert!(Slot::take(&live, MAX_CONNECTIONS).is_none(), "the bound admits no more");
        assert_eq!(live.load(Ordering::Relaxed), MAX_CONNECTIONS);
        drop(slots);
        assert_eq!(live.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn a_connection_is_carried_both_ways_with_its_half_close() {
        use std::os::unix::net::UnixListener;
        let dir = std::env::temp_dir().join(format!("forward-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("proxy.sock");
        let proxy = UnixListener::bind(&socket).unwrap();
        let echo = std::thread::spawn(move || {
            let (mut stream, _) = proxy.accept().unwrap();
            let mut asked = Vec::new();
            stream.read_to_end(&mut asked).unwrap();
            stream.write_all(&[b"echo:".as_slice(), &asked].concat()).unwrap();
        });
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (accepted, _) = listener.accept().unwrap();
        let gate = Gate::new(dir.to_str().unwrap());
        let forwarder = std::thread::spawn(move || serve(accepted, socket.to_str().unwrap(), &gate));
        client.write_all(b"ping").unwrap();
        client.shutdown(Shutdown::Write).unwrap();
        let mut answer = Vec::new();
        client.read_to_end(&mut answer).unwrap();
        assert_eq!(answer, b"echo:ping");
        echo.join().unwrap();
        forwarder.join().unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_cap_fits_the_descriptor_limit() {
        // Docker's default soft limit leaves room for the whole bound, as does no limit at all.
        assert_eq!(connection_cap(Some(1024)), Some(MAX_CONNECTIONS));
        assert_eq!(connection_cap(None), Some(MAX_CONNECTIONS));
        assert_eq!(connection_cap(Some(512)), Some(128));
        assert_eq!(connection_cap(Some(256 + 2 * 32)), Some(32));
        assert_eq!(connection_cap(Some(256 + 2 * 32 - 1)), None, "fewer than Chromium needs");
        assert_eq!(connection_cap(Some(0)), None);
    }

    #[test]
    fn a_slot_returns_when_its_thread_unwinds() {
        let live = Arc::new(AtomicUsize::new(0));
        let slot = Slot::take(&live, MAX_CONNECTIONS).expect("a free slot");
        let thread = std::thread::spawn(move || {
            let _slot = slot;
            panic!("the connection's thread failed");
        });
        assert!(thread.join().is_err());
        assert_eq!(live.load(Ordering::Relaxed), 0);
    }
}
