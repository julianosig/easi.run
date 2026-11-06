//! A session's CDP relay (`PROXY_RELAY_LISTEN` and `PROXY_RELAY_SOCKET`): each TCP connection to its address is carried
//! unchanged to the browser's DevTools socket, which the browser's launcher serves, and nowhere else. It reads nothing of
//! what it carries and decides nothing: the destination is fixed by its configuration, so a client chooses none. Docker
//! publishes its one port on the host's loopback.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use tokio::net::{TcpListener, TcpStream, UnixStream};
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::Semaphore;

/// Connections at once: Playwright, Puppeteer and Selenium each open one or two; more wait for a free one.
const MAX_CONNECTIONS: usize = 32;

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

/// The relay's address and socket, when this run is a relay; both go together, and nothing else configures it.
pub fn configured() -> Result<Option<(SocketAddr, PathBuf)>, String> {
    match (env("PROXY_RELAY_LISTEN"), env("PROXY_RELAY_SOCKET")) {
        (None, None) => Ok(None),
        (Some(listen), Some(socket)) => {
            let others = std::env::vars().any(|(name, value)| {
                name.starts_with("PROXY_") && !name.starts_with("PROXY_RELAY_") && !value.is_empty()
            });
            if others {
                return Err("PROXY_RELAY_LISTEN and PROXY_RELAY_SOCKET go alone".into());
            }
            let listen =
                listen.parse().map_err(|_| format!("PROXY_RELAY_LISTEN: invalid socket address {listen:?}"))?;
            Ok(Some((listen, socket.into())))
        }
        _ => Err("PROXY_RELAY_LISTEN and PROXY_RELAY_SOCKET go together".into()),
    }
}

/// Carries one connection both ways until either side ends it.
async fn carry(mut client: TcpStream, socket: &std::path::Path) {
    let _ = client.set_nodelay(true);
    let Ok(mut browser) = UnixStream::connect(socket).await else { return };
    let _ = tokio::io::copy_bidirectional(&mut client, &mut browser).await;
}

/// Serves until a signal (success) or a failed listener (failure).
pub fn run(listen: SocketAddr, socket: PathBuf) -> Result<ExitCode, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("cannot start the runtime: {error}"))?;
    runtime.block_on(async move {
        let (Ok(mut term), Ok(mut int)) = (signal(SignalKind::terminate()), signal(SignalKind::interrupt())) else {
            return Err("cannot install signal handlers".to_owned());
        };
        let listener =
            TcpListener::bind(listen).await.map_err(|error| format!("cannot listen on {listen}: {error}"))?;
        eprintln!("proxy: relaying {listen} to {}", socket.display());
        let serve = serve(listener, Arc::new(socket));
        Ok(tokio::select! {
            () = serve => ExitCode::FAILURE,
            _ = term.recv() => ExitCode::SUCCESS,
            _ = int.recv() => ExitCode::SUCCESS,
        })
    })
}

async fn serve(listener: TcpListener, socket: Arc<PathBuf>) {
    let slots = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    loop {
        let Ok(slot) = slots.clone().acquire_owned().await else { return };
        let Ok((client, _)) = listener.accept().await else {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            continue;
        };
        let socket = socket.clone();
        tokio::spawn(async move {
            carry(client, &socket).await;
            drop(slot);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn a_connection_reaches_the_socket_unchanged_both_ways() {
        let dir = std::env::temp_dir().join(format!("relay-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let socket = dir.join("cdp.sock");
        let _ = std::fs::remove_file(&socket);
        let browser = tokio::net::UnixListener::bind(&socket).unwrap();
        let echo = tokio::spawn(async move {
            let (mut stream, _) = browser.accept().await.unwrap();
            let mut asked = [0u8; 4];
            stream.read_exact(&mut asked).await.unwrap();
            stream.write_all(&[b"cdp:".as_slice(), &asked].concat()).await.unwrap();
        });
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(serve(listener, Arc::new(socket)));
        let mut client = TcpStream::connect(address).await.unwrap();
        client.write_all(b"ping").await.unwrap();
        let mut answer = [0u8; 8];
        client.read_exact(&mut answer).await.unwrap();
        assert_eq!(&answer, b"cdp:ping");
        echo.await.unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }
}
