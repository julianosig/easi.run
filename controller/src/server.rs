//! The Unix-socket front: admin uid only, one JSON request per line, one JSON response per line, bounded.

use std::path::Path;
use std::sync::Arc;

use bollard::container::LogOutput;
use futures_util::StreamExt;
use line::Line;
use serde_json::{Value, json};
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Semaphore;

use crate::ops::{Controller, ExecInput, ExecOutput};
use crate::request::Request;
use crate::template::Exec;

pub const MAX_LINE: usize = 64 * 1024;
/// Connections at once. An agent link holds one for its session's life (the admin runs up to 16 sessions), so the rest
/// leave room beside them for each session's inspection and control operations at once.
pub const MAX_CONNECTIONS: usize = 64;
/// At most this much of one exec stderr chunk reaches the controller's log.
const STDERR_LOG: usize = 512;
/// What one exec's stderr may put in the controller's log in all, in bytes. Each line also costs [`STDERR_LINE_COST`]
/// for its prefix, so a stream of tiny writes is bounded too; past the budget, one line says the rest was cut.
const STDERR_BUDGET: usize = 64 * 1024;
const STDERR_LINE_COST: usize = 128;

/// What becomes of one stderr chunk of an exec.
#[derive(Debug, PartialEq, Eq)]
enum Stderr<'a> {
    /// Logged, cut to [`STDERR_LOG`].
    Log(&'a [u8]),
    /// The first chunk past the budget: it and everything after are not logged, which one line says.
    Truncated,
    /// Not logged.
    Drop,
}

/// One exec's stderr budget: what was logged and how many bytes were not.
struct StderrLog {
    left: usize,
    spent: bool,
    dropped: u64,
}

impl StderrLog {
    fn new() -> Self {
        Self { left: STDERR_BUDGET, spent: false, dropped: 0 }
    }

    fn take<'a>(&mut self, message: &'a [u8]) -> Stderr<'a> {
        let shown = &message[..message.len().min(STDERR_LOG)];
        let cost = shown.len() + STDERR_LINE_COST;
        if !self.spent && cost <= self.left {
            self.left -= cost;
            self.dropped += (message.len() - shown.len()) as u64;
            return Stderr::Log(shown);
        }
        self.dropped += message.len() as u64;
        if self.spent {
            return Stderr::Drop;
        }
        self.spent = true;
        Stderr::Truncated
    }
}

/// Binds the socket with mode 0660 and the group of its directory (the `controller` volume's group, which the admin
/// holds). A stale socket from an earlier run is replaced; any other file at the path is left alone and refused.
pub fn bind(path: &Path) -> Result<UnixListener, String> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
    let shown = path.display();
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_socket() => {
            std::fs::remove_file(path).map_err(|e| format!("{shown}: {e}"))?;
        }
        Ok(_) => return Err(format!("{shown} exists and is not a socket")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("{shown}: {e}")),
    }
    let dir = path.parent().ok_or(format!("{shown} has no directory"))?;
    let gid = std::fs::metadata(dir).map_err(|e| format!("{}: {e}", dir.display()))?.gid();
    let listener = UnixListener::bind(path).map_err(|e| format!("{shown}: {e}"))?;
    std::os::unix::fs::chown(path, None, Some(gid)).map_err(|e| format!("{shown}: chown: {e}"))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o660)).map_err(|e| format!("{shown}: {e}"))?;
    Ok(listener)
}

pub async fn serve(listener: UnixListener, controller: Arc<Controller>, admin_uid: u32) {
    let slots = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    loop {
        let stream = match listener.accept().await {
            Ok((stream, _)) => stream,
            Err(e) => {
                tracing::warn!(%e, "accept failed");
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                continue;
            }
        };
        // `UnixStream::peer_cred` reads SO_PEERCRED without judging the pid: across PID namespaces (the admin is in
        // another container) the kernel reports pid 0, which a check that insists on a pid would refuse.
        let peer = stream.peer_cred().ok().map(|cred| cred.uid());
        if peer != Some(admin_uid) {
            tracing::warn!(?peer, "refused a connection from another uid");
            continue;
        }
        let Ok(slot) = slots.clone().try_acquire_owned() else {
            tracing::warn!("refused a connection: too many open");
            tokio::spawn(async move {
                let mut stream = stream;
                let _ = stream.write_all(b"{\"ok\":false,\"error\":\"too many connections\"}\n").await;
            });
            continue;
        };
        let controller = controller.clone();
        tokio::spawn(async move {
            connection(stream, controller).await;
            drop(slot);
        });
    }
}

fn refusal(error: impl Into<String>) -> Value {
    json!({ "ok": false, "error": error.into() })
}

async fn respond(stream: &mut BufReader<UnixStream>, value: &Value) -> std::io::Result<()> {
    let mut bytes = serde_json::to_vec(value).unwrap_or_else(|_| b"{\"ok\":false}".to_vec());
    bytes.push(b'\n');
    stream.get_mut().write_all(&bytes).await
}

async fn connection(stream: UnixStream, controller: Arc<Controller>) {
    let mut stream = BufReader::with_capacity(MAX_LINE, stream);
    loop {
        let line = match line::read(&mut stream, MAX_LINE).await {
            Ok(Line::Complete(line)) => line,
            Ok(Line::Closed) | Err(_) => return,
            Ok(Line::TooLong) => {
                tracing::warn!("refused a request longer than {MAX_LINE} bytes");
                let _ = respond(&mut stream, &refusal("request too long")).await;
                return;
            }
        };
        let request = match Request::parse(&line) {
            Ok(request) => request,
            Err(why) => {
                tracing::warn!(%why, "refused a request outside the closed set");
                if respond(&mut stream, &refusal(format!("refused: {why}"))).await.is_err() {
                    return;
                }
                continue;
            }
        };
        let op = request.name();
        let answer = match request {
            Request::CreateSession(create) => match create.validate() {
                Ok(spec) => controller.create_session(spec).await,
                Err(why) => Err(format!("refused: {why}")),
            },
            Request::StopSession(r) => controller.stop_session(&r.session).await,
            Request::InspectSession(r) => controller.inspect_session(&r.session).await,
            Request::AgentTerminate(r) => controller.agent_terminate(&r.session).await,
            Request::AgentRestart(r) => controller.agent_restart(&r.session).await,
            Request::Cleanup(_) => controller.cleanup().await,
            Request::AgentAttach(r) => match controller.agent_attach(&r.session).await {
                Ok((exec, output, input)) => {
                    if respond(&mut stream, &json!({ "ok": true, "exec": exec })).await.is_ok() {
                        relay(stream, output, input, Until::Either, "agent").await;
                    }
                    tracing::info!(session = %r.session, exec, "agent relay ended");
                    return;
                }
                Err(why) => Err(why),
            },
            Request::ProfileRestore(r) | Request::ProfileSave(r) => {
                let exec = if op == "profile_restore" { Exec::ProfileRestore } else { Exec::ProfileSave };
                match controller.profile_exec(&r.session, exec).await {
                    Ok((exec, output, input)) => {
                        if respond(&mut stream, &json!({ "ok": true, "exec": exec })).await.is_ok() {
                            relay(stream, output, input, Until::Output, "browser profile").await;
                        }
                        tracing::info!(session = %r.session, exec, "profile relay ended");
                        return;
                    }
                    Err(why) => Err(why),
                }
            }
        };
        let value = answer.unwrap_or_else(|why| {
            tracing::warn!(op, %why, "operation refused or failed");
            refusal(why)
        });
        if respond(&mut stream, &value).await.is_err() {
            return;
        }
    }
}

/// Logs one stderr chunk of `what` (the agent, or a browser's profile command) as one line. Neither is trusted, and
/// their errors quote page text or file names a page chose: the chunk is escaped (`Debug`), so a newline, a carriage
/// return, or an escape sequence in it can neither forge a log line nor reach the terminal of whoever reads the log.
fn log_stderr(what: &'static str, shown: &[u8]) {
    tracing::info!(stderr = ?String::from_utf8_lossy(shown).trim_end(), "{what}");
}

/// When a relay ends.
#[derive(Clone, Copy)]
enum Until {
    /// As soon as either side closes: the agent's link.
    Either,
    /// Once the exec's output ends: a profile command still answers after the admin finished sending (a restore's
    /// result), so the admin's end of the input closes only the exec's standard input.
    Output,
}

/// Relays raw bytes between the connection and the exec's stdio until `until` says it is over.
async fn relay(
    stream: BufReader<UnixStream>,
    mut output: ExecOutput,
    mut input: ExecInput,
    until: Until,
    what: &'static str,
) {
    let pending = stream.buffer().to_vec();
    let (mut from_client, mut to_client) = stream.into_inner().into_split();
    let mut stderr = StderrLog::new();
    let upstream = async {
        if !pending.is_empty() {
            input.write_all(&pending).await?;
        }
        tokio::io::copy(&mut from_client, &mut input).await?;
        input.shutdown().await
    };
    let downstream = async {
        while let Some(chunk) = output.next().await {
            match chunk {
                Ok(LogOutput::StdOut { message }) => to_client.write_all(&message).await?,
                Ok(LogOutput::StdErr { message }) => match stderr.take(&message) {
                    Stderr::Log(shown) => log_stderr(what, shown),
                    Stderr::Truncated => {
                        tracing::warn!("{what} stderr truncated: the rest of this exec's stderr is not logged")
                    }
                    Stderr::Drop => {}
                },
                Ok(_) => {}
                Err(e) => return Err(std::io::Error::other(e)),
            }
        }
        to_client.shutdown().await
    };
    let upstream = async {
        let result = upstream.await;
        if let Err(e) = &result {
            tracing::debug!(%e, "relay to the exec ended");
        }
        if matches!(until, Until::Output) {
            std::future::pending::<()>().await;
        }
    };
    tokio::select! {
        () = upstream => {}
        result = downstream => if let Err(e) = result { tracing::debug!(%e, "relay from the exec ended") },
    }
    if stderr.dropped > 0 {
        tracing::info!(bytes = stderr.dropped, "{what} stderr not logged");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_execs_stderr_reaches_the_log_within_its_budget() {
        // Tiny writes: each line costs its prefix, so the count of lines is bounded as well as their bytes.
        let mut log = StderrLog::new();
        let outcomes: Vec<_> = (0..100_000).map(|_| log.take(b"x")).collect();
        let logged = outcomes.iter().filter(|o| matches!(o, Stderr::Log(_))).count();
        assert_eq!(logged, STDERR_BUDGET / (1 + STDERR_LINE_COST));
        assert_eq!(outcomes[logged], Stderr::Truncated, "one line says the rest was cut");
        assert!(outcomes[logged + 1..].iter().all(|o| *o == Stderr::Drop));
        assert_eq!(log.dropped, (100_000 - logged) as u64);

        // Large chunks: each is cut to its first STDERR_LOG bytes, and the budget still holds.
        let mut log = StderrLog::new();
        let chunk = vec![b'y'; 10_000];
        let mut spent = 0;
        let mut truncated = 0;
        for _ in 0..1000 {
            match log.take(&chunk) {
                Stderr::Log(shown) => {
                    assert_eq!(shown.len(), STDERR_LOG);
                    spent += shown.len() + STDERR_LINE_COST;
                }
                Stderr::Truncated => truncated += 1,
                Stderr::Drop => {}
            }
        }
        assert!(spent <= STDERR_BUDGET && STDERR_BUDGET - spent < STDERR_LOG + STDERR_LINE_COST, "{spent}");
        assert_eq!(truncated, 1);
        let logged_bytes = (spent / (STDERR_LOG + STDERR_LINE_COST) * STDERR_LOG) as u64;
        assert_eq!(log.dropped, 1000 * 10_000 - logged_bytes);
    }

    #[test]
    fn an_execs_stderr_is_logged_as_one_escaped_line() {
        use std::sync::{Arc, Mutex};

        #[derive(Clone, Default)]
        struct Captured(Arc<Mutex<Vec<u8>>>);
        impl std::io::Write for Captured {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let captured = Captured::default();
        let writer = captured.clone();
        let subscriber = tracing_subscriber::fmt().with_ansi(false).with_writer(move || writer.clone()).finish();
        let hostile =
            "x\n2026-10-23T12:00:00.000000Z  INFO controller::ops: session stopped\r\x1b]0;title\x07\x1b[2J\u{9b}2J\n";
        tracing::subscriber::with_default(subscriber, || log_stderr("agent", hostile.as_bytes()));
        let out = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
        assert_eq!(out.matches('\n').count(), 1, "{out:?}");
        assert!(out.ends_with('\n'), "{out:?}");
        assert!(!out.chars().any(|c| c != '\n' && c.is_control()), "a raw control character was logged: {out:?}");
        assert!(out.contains(r"x\n2026-10-23") && out.contains(r"\u{1b}[2J"), "{out:?}");
    }
}
