//! One `cua-inject v1` line for a session's automation agent, sent to its display on a link of its own (see
//! `frame::Link::Inject`): the display serves it as it serves the agent's own injection socket, refused while a person
//! holds control, and answers once its effect is in.

use std::path::Path;
use std::time::Duration;

use display_protocol::Direction;
use display_protocol::frame::{FrameReader, Link, MAX_CHUNK, MAX_INJECTION};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

/// How long a line may take to be answered: the longest text the display types for the agent (some 340 characters
/// outside the keymap, each a keymap switch) takes about a second, and a window that stops reading holds it two more.
const ANSWER_DEADLINE: Duration = Duration::from_secs(10);

/// Sends `line` to the display whose stream socket is `path`, and returns its answer (`ok`, or `err` and why). A line
/// no link carries (see `frame::injection`) is refused here, before anything is sent.
pub async fn send(path: &Path, line: String) -> Result<String, String> {
    if !display_protocol::frame::injection(&line) {
        return Err(format!("an injection line of {} bytes, past the link's {}", line.len(), MAX_INJECTION));
    }
    let exchange = async {
        let mut stream = UnixStream::connect(path).await.map_err(|e| format!("display: {e}"))?;
        stream.write_all(&Link::Inject { line }.encode()).await.map_err(|e| format!("display: {e}"))?;
        let mut frames = FrameReader::new(Direction::ServerToClient);
        let mut chunk = vec![0u8; MAX_CHUNK];
        loop {
            let n = stream.read(&mut chunk).await.map_err(|e| format!("display: {e}"))?;
            if n == 0 {
                return Err("the display closed its link before it answered".to_owned());
            }
            frames.push(&chunk[..n]).map_err(|e| format!("display: {e}"))?;
            while let Some(body) = frames.next_body().map_err(|e| format!("display: {e}"))? {
                if let Ok(Link::Injected { answer }) = Link::decode(&body, Direction::ServerToClient) {
                    return Ok(answer);
                }
            }
        }
    };
    tokio::time::timeout(ANSWER_DEADLINE, exchange)
        .await
        .unwrap_or_else(|_| Err("the display did not answer in time".into()))
}

/// A display for tests on a Unix socket at `path`: it records each line it is sent and answers it with what `answer`
/// says for it.
#[cfg(test)]
pub fn fake(
    path: std::path::PathBuf,
    answer: impl Fn(&str) -> String + Send + Sync + 'static,
) -> std::sync::Arc<std::sync::Mutex<Vec<String>>> {
    let lines = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let (recorded, answer) = (lines.clone(), std::sync::Arc::new(answer));
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let (recorded, answer) = (recorded.clone(), answer.clone());
            tokio::spawn(async move {
                let mut frames = FrameReader::new(Direction::ClientToServer);
                let mut chunk = vec![0u8; MAX_CHUNK];
                while let Ok(n @ 1..) = stream.read(&mut chunk).await {
                    frames.push(&chunk[..n]).unwrap();
                    while let Some(body) = frames.next_body().unwrap() {
                        let Ok(Link::Inject { line }) = Link::decode(&body, Direction::ClientToServer) else { return };
                        let reply = Link::Injected { answer: answer(&line) };
                        recorded.lock().unwrap().push(line);
                        stream.write_all(&reply.encode()).await.unwrap();
                    }
                }
            });
        }
    });
    lines
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn a_line_no_link_carries_is_refused_before_anything_is_sent() {
        let dir = crate::private::test_dir("inject-long");
        let lines = super::fake(dir.join("display.sock"), |_| "ok".to_owned());
        let long = format!("u root:7 {}", "c3a9".repeat(600));
        let refused = super::send(&dir.join("display.sock"), long).await;
        assert!(refused.is_err_and(|why| why.contains("past the link's")));
        assert!(super::send(&dir.join("display.sock"), "u root:7 \n".into()).await.is_err());
        assert!(lines.lock().unwrap().is_empty());
    }
}
