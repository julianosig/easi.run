//! The proxy forwarder: the browser has no network (its own namespace's link reaches nothing), so Chromium speaks to
//! its proxy at a loopback address and each connection is carried unchanged to the proxy's Unix socket. It decides nothing: the proxy vets every
//! request. Connections are bounded, and what comes from the network waits at the downloads' gate (see `gate`).

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::gate::Gate;

const MAX_CONNECTIONS: usize = 256;

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
    fn take(live: &Arc<AtomicUsize>) -> Option<Self> {
        if live.fetch_add(1, Ordering::Relaxed) >= MAX_CONNECTIONS {
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

fn serve(client: TcpStream, socket: &str, gate: &Gate) {
    let Ok(upstream) = UnixStream::connect(socket) else { return };
    let (Ok(client_read), Ok(upstream_read), Ok(upstream_write)) =
        (client.try_clone(), upstream.try_clone(), upstream.try_clone())
    else {
        return;
    };
    let up = std::thread::Builder::new().stack_size(64 * 1024).spawn(move || {
        copy(client_read, &upstream_write, None, || {
            let _ = upstream_write.shutdown(Shutdown::Write);
        });
    });
    // Without the second thread the connection cannot be carried; dropping both ends here closes it.
    let Ok(up) = up else { return };
    copy(upstream_read, &client, Some(gate), || {
        let _ = client.shutdown(Shutdown::Write);
    });
    let _ = up.join();
}

/// Serves `listen` until the process ends, on its own thread.
pub fn start(listen: &str, socket: &str, gate: Arc<Gate>) -> std::io::Result<()> {
    let listener = TcpListener::bind(listen)?;
    let socket: Arc<str> = socket.into();
    std::thread::Builder::new().name("proxy-forward".into()).spawn(move || {
        let live = Arc::new(AtomicUsize::new(0));
        for client in listener.incoming().flatten() {
            let Some(slot) = Slot::take(&live) else { continue };
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
        let slots: Vec<_> = (0..MAX_CONNECTIONS).map(|_| Slot::take(&live).expect("a free slot")).collect();
        assert!(Slot::take(&live).is_none(), "the bound admits no more");
        assert_eq!(live.load(Ordering::Relaxed), MAX_CONNECTIONS);
        drop(slots);
        assert_eq!(live.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn a_slot_returns_when_its_thread_unwinds() {
        let live = Arc::new(AtomicUsize::new(0));
        let slot = Slot::take(&live).expect("a free slot");
        let thread = std::thread::spawn(move || {
            let _slot = slot;
            panic!("the connection's thread failed");
        });
        assert!(thread.join().is_err());
        assert_eq!(live.load(Ordering::Relaxed), 0);
    }
}
