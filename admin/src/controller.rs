//! The controller's closed operation set, over its Unix socket: one JSON request line, one JSON response line.

use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use crate::app::Locale;

const DEADLINE: Duration = Duration::from_secs(120);
/// How long reaching the controller and writing a request may take.
const SEND_DEADLINE: Duration = Duration::from_secs(10);
const MAX_RESPONSE: usize = 256 * 1024;
/// The read buffer of a connection to the controller: an 11 MiB answer on the agent's link took 10-25% less time to
/// read through 64 KiB than through `BufReader`'s default 8 KiB, and 256 KiB gained little more.
const LINK_BUFFER: usize = 64 * 1024;

/// What a new session is to be.
pub struct NewSession<'a> {
    pub session: &'a str,
    pub url: &'a str,
    pub devtools: bool,
    pub size: (u32, u32),
    pub locale: &'a Locale,
    pub lane: Option<u8>,
    pub egress: Option<&'a egress::Upstream>,
    pub kiosk: bool,
    /// The controller must enforce the walled garden (the public demo).
    pub walled: bool,
    /// The session starts from a saved profile (see `profiles`).
    pub profile: bool,
}

#[derive(Clone)]
pub struct Controller {
    socket: PathBuf,
}

impl Controller {
    pub fn new(socket: PathBuf) -> Controller {
        Controller { socket }
    }

    async fn request(&self, body: Value) -> Result<(Value, BufReader<UnixStream>), String> {
        let mut line = serde_json::to_vec(&body).map_err(|e| e.to_string())?;
        line.push(b'\n');
        // Reaching the controller and handing it the request are bounded too, not only its answer.
        let sent = tokio::time::timeout(SEND_DEADLINE, async {
            let mut stream = UnixStream::connect(&self.socket).await?;
            stream.write_all(&line).await?;
            Ok::<_, std::io::Error>(stream)
        })
        .await
        .map_err(|_| "controller: not reached in time".to_owned())?
        .map_err(|e| format!("controller: {e}"))?;
        // The agent's link reads its answers, up to 16 MiB a line, through this buffer (see `agent_attach`); it is made
        // here because it may already hold bytes past the response line.
        let mut stream = BufReader::with_capacity(LINK_BUFFER, sent);
        let mut response = String::new();
        let read = tokio::time::timeout(DEADLINE, (&mut stream).take(MAX_RESPONSE as u64).read_line(&mut response))
            .await
            .map_err(|_| "controller: no answer in time".to_owned())?
            .map_err(|e| format!("controller: {e}"))?;
        if read == 0 {
            return Err("controller: closed without answering".into());
        }
        let value: Value = serde_json::from_str(&response).map_err(|e| format!("controller: {e}"))?;
        if value.get("ok") != Some(&Value::Bool(true)) {
            let why = value.get("error").and_then(Value::as_str).unwrap_or("refused").to_owned();
            return Err(format!("controller: {why}"));
        }
        Ok((value, stream))
    }

    pub async fn create_session(&self, new: &NewSession<'_>) -> Result<(), String> {
        let (width, height) = new.size;
        let mut op = json!({
            "op": "create_session", "session": new.session, "url": new.url, "devtools": new.devtools,
            "width": width, "height": height, "timezone": new.locale.timezone, "languages": new.locale.languages,
        });
        if let Some(lane) = new.lane {
            op["lane"] = json!(lane);
        }
        if let Some(egress) = new.egress {
            op["egress"] = json!(egress);
        }
        if new.kiosk {
            op["kiosk"] = json!(true);
        }
        if new.walled {
            op["walled"] = json!(true);
        }
        if new.profile {
            op["profile"] = json!(true);
        }
        self.request(op).await.map(|_| ())
    }

    /// Stops a session; one the controller does not hold is reclaimed by its label, so a retried stop succeeds.
    pub async fn stop_session(&self, session: &str) -> Result<(), String> {
        self.request(json!({"op": "stop_session", "session": session})).await.map(|_| ())
    }

    pub async fn inspect_session(&self, session: &str) -> Result<Value, String> {
        self.request(json!({"op": "inspect_session", "session": session})).await.map(|(v, _)| v)
    }

    /// Starts the session's one agent exec and returns the stream that now carries its standard streams.
    pub async fn agent_attach(&self, session: &str) -> Result<BufReader<UnixStream>, String> {
        self.request(json!({"op": "agent_attach", "session": session})).await.map(|(_, stream)| stream)
    }

    /// Kills the agent container; succeeds only with the controller's proof that it no longer runs.
    pub async fn agent_terminate(&self, session: &str) -> Result<(), String> {
        let (value, _) = self.request(json!({"op": "agent_terminate", "session": session})).await?;
        if value.get("exited") == Some(&Value::Bool(true)) {
            Ok(())
        } else {
            Err("controller: the agent was not proven stopped".into())
        }
    }

    pub async fn agent_restart(&self, session: &str) -> Result<(), String> {
        self.request(json!({"op": "agent_restart", "session": session})).await.map(|_| ())
    }

    /// Starts the session's `profile-restore` (see `profiles`) and returns the stream that now carries its standard
    /// streams: the archive to it, its answer back.
    pub async fn profile_restore(&self, session: &str) -> Result<BufReader<UnixStream>, String> {
        self.request(json!({"op": "profile_restore", "session": session})).await.map(|(_, stream)| stream)
    }

    /// Starts the session's `profile-save` and returns the stream that now carries the profile back.
    pub async fn profile_save(&self, session: &str) -> Result<BufReader<UnixStream>, String> {
        self.request(json!({"op": "profile_save", "session": session})).await.map(|(_, stream)| stream)
    }

    pub async fn cleanup(&self) -> Result<Value, String> {
        self.request(json!({"op": "cleanup"})).await.map(|(v, _)| v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_agent_s_link_reads_through_a_large_buffer_that_keeps_what_came_past_the_response() {
        let dir = crate::private::test_dir("controller-link");
        let path = dir.join("controller.sock");
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        let next = format!("{}\n", "a".repeat(32 * 1024));
        let sent = format!("{{\"ok\":true}}\n{next}");
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = vec![0u8; 256];
            let _ = stream.read(&mut request).await;
            stream.write_all(sent.as_bytes()).await.unwrap();
            std::future::pending::<()>().await;
        });
        let mut link = Controller::new(path).agent_attach("abc").await.unwrap();
        // One read took more than the default 8 KiB.
        assert!(link.buffer().len() > 8 * 1024, "{} bytes buffered", link.buffer().len());
        let mut read = String::new();
        link.read_line(&mut read).await.unwrap();
        assert_eq!(read, next);
    }
}
