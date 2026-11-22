//! A link of its own to a session's display (its stream socket): for one question and its answer (an injection line,
//! `inject`; a reading of the page, a script or a navigation, `inspect`), or kept, as the agent's files link
//! (`agent_files`).

use std::path::Path;

use display_protocol::Direction;
use display_protocol::frame::{FrameReader, Link, MAX_CHUNK};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

pub struct DisplayLink {
    stream: UnixStream,
    frames: FrameReader,
    chunk: Vec<u8>,
    /// What was no frame, after links of the same read that were: said at the next read.
    broken: Option<String>,
}

fn failed(e: impl std::fmt::Display) -> String {
    format!("display: {e}")
}

impl DisplayLink {
    pub async fn open(path: &Path) -> Result<DisplayLink, String> {
        let stream = UnixStream::connect(path).await.map_err(failed)?;
        let frames = FrameReader::new(Direction::ServerToClient);
        Ok(DisplayLink { stream, frames, chunk: vec![0u8; MAX_CHUNK], broken: None })
    }

    /// Opens a link to the display at `path`, sends it `question`, and waits for the answer `pick` finds among what
    /// it sends, within `deadline` (`late` says why there is none past it).
    pub async fn ask(
        path: &Path,
        question: &Link,
        pick: impl Fn(Link) -> Option<String>,
        deadline: std::time::Duration,
        late: &str,
    ) -> Result<String, String> {
        let exchange = async {
            let mut link = DisplayLink::open(path).await?;
            link.send(question).await?;
            loop {
                if let Some(answer) = link.read().await?.into_iter().find_map(&pick) {
                    return Ok(answer);
                }
            }
        };
        tokio::time::timeout(deadline, exchange).await.unwrap_or_else(|_| Err(late.to_owned()))
    }

    /// What a read found that is no frame, after links of the same read it gave: said now rather than at the next read,
    /// for a link that does more between its reads than wait for an answer.
    pub fn broken(&mut self) -> Result<(), String> {
        self.broken.take().map_or(Ok(()), Err)
    }

    pub async fn send(&mut self, link: &Link) -> Result<(), String> {
        self.stream.write_all(&link.encode()).await.map_err(failed)
    }

    /// Reads once, and gives the links that read made whole that read as one (any other is passed over), or why
    /// there are none: the display closed the link, or sent what is no frame. What is no frame after links that are
    /// is said at the next read, once those links are given. Dropped while it waits, as in a `select!`, it has read
    /// nothing.
    pub async fn read(&mut self) -> Result<Vec<Link>, String> {
        if let Some(why) = self.broken.take() {
            return Err(why);
        }
        let n = self.stream.read(&mut self.chunk).await.map_err(failed)?;
        if n == 0 {
            return Err("the display closed its link before it answered".to_owned());
        }
        self.frames.push(&self.chunk[..n]).map_err(failed)?;
        let mut links = Vec::new();
        loop {
            match self.frames.next_body() {
                Ok(Some(body)) => links.extend(Link::decode(&body, Direction::ServerToClient).ok()),
                Ok(None) => return Ok(links),
                Err(e) if links.is_empty() => return Err(failed(e)),
                Err(e) => {
                    self.broken = Some(failed(e));
                    return Ok(links);
                }
            }
        }
    }
}
