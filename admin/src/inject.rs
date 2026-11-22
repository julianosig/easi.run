//! One `cua-inject v1` line for a session's automation agent, sent to its display on a link of its own (see
//! `frame::Link::Inject`): the display serves it as it serves the agent's own injection socket, refused while a person
//! holds control, and answers once its own input went: for keys, once the launcher's checks that the browser handled
//! them (which infer it, as measured) are done, or when the display's wait for them ran out, or at once for a browser
//! that cannot run them (see the display's `input::settle`), so an answer is no proof the keys' effect (a focus moved)
//! is in.

use std::path::Path;
use std::time::Duration;

use display_protocol::frame::{Link, MAX_INJECTION};

/// How long a line may take to be answered: the display answers it once the text typed before it is in, its own
/// within the display's budgets for a line (`input::TYPING_BUDGET`, `input::LINE_BUDGET`: inside the driver's 5 s
/// wait), so this leaves room for a line waiting behind another.
const ANSWER_DEADLINE: Duration = Duration::from_secs(10);

/// Sends `line` to the display whose stream socket is `path`, and returns its answer (`ok`, or `err` and why). A line
/// no link carries (see `frame::injection`) is refused here, before anything is sent.
pub async fn send(path: &Path, line: String) -> Result<String, String> {
    if !display_protocol::frame::injection(&line) {
        return Err(format!("an injection line of {} bytes, past the link's {}", line.len(), MAX_INJECTION));
    }
    let injected = |link| if let Link::Injected { answer } = link { Some(answer) } else { None };
    let late = "the display did not answer in time";
    crate::display_link::DisplayLink::ask(path, &Link::Inject { line }, injected, ANSWER_DEADLINE, late).await
}

/// A display for tests on a Unix socket at `path`: it records each line it is sent and answers it with what `answer`
/// says for it.
#[cfg(test)]
pub fn fake(
    path: std::path::PathBuf,
    answer: impl Fn(&str) -> String + Send + Sync + 'static,
) -> std::sync::Arc<std::sync::Mutex<Vec<String>>> {
    use display_protocol::Direction;
    use display_protocol::frame::{FrameReader, MAX_CHUNK};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
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
