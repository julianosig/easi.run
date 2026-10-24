//! The local MCP socket, `mcp.sock` in the state directory, and `admin mcp`, which relays a client's stdio to it.
//!
//! A client runs `docker exec -i <admin container> admin mcp` (over `ssh -T` from another machine): the relay copies
//! its stdin to the socket and the socket to its stdout, byte for byte, and says anything else on stderr only. The
//! running admin serves the socket with the MCP endpoint's own handling (see `mcp`), one JSON-RPC message per line and
//! no headers, under one principal, `Owner::Local`, shared by every client: it has exactly an agent token's rights,
//! reaching only the sessions local clients started. No token, port or certificate is involved: the socket is mode
//! 0600 in the admin's own 0700 state directory, and the admin answers only a peer with its own uid, so only a process
//! running as the admin's uid with that directory mounted (in practice, whoever may run commands in the admin's
//! container, which already takes Docker access) reaches it. `ADMIN_LOCAL_MCP=0` turns it off; the demo never serves it.

use std::path::Path;
use std::sync::Arc;

use serde_json::Value;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Semaphore, mpsc};

use crate::api::Shared;
use crate::app::Owner;
use crate::mcp::{self, Reply, Transport};

/// The socket's name in the state directory.
pub const SOCKET: &str = "mcp.sock";
/// Connections at once: each is one client's session.
const MAX_CONNECTIONS: usize = 16;
/// Requests one connection may have in progress at once; past them, its next line waits to be read.
const IN_FLIGHT: usize = 8;
/// The longest message, in bytes: the HTTP endpoint's body limit.
const MAX_LINE: usize = crate::tools::BODY_LIMIT;

/// Binds the socket at `path`, mode 0600. A socket an earlier run left (nothing answers on it) is replaced; one another
/// admin still serves, or any other file at the path, is left alone and refused.
pub fn bind(path: &Path) -> Result<UnixListener, String> {
    use std::os::unix::fs::{FileTypeExt, PermissionsExt};
    let shown = path.display();
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_socket() => match std::os::unix::net::UnixStream::connect(path) {
            Ok(_) => return Err(format!("{shown}: another admin serves this state directory")),
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => {
                std::fs::remove_file(path).map_err(|e| format!("{shown}: {e}"))?;
            }
            Err(e) => return Err(format!("{shown}: {e}")),
        },
        Ok(_) => return Err(format!("{shown} exists and is not a socket")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("{shown}: {e}")),
    }
    let listener = UnixListener::bind(path).map_err(|e| format!("{shown}: {e}"))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|e| format!("{shown}: {e}"))?;
    Ok(listener)
}

/// Serves the socket for as long as the admin runs: each connection from the admin's own uid, up to
/// [`MAX_CONNECTIONS`] at once.
pub async fn serve(listener: UnixListener, app: Shared) {
    let uid = rustix::process::getuid().as_raw();
    let slots = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    loop {
        let stream = match listener.accept().await {
            Ok((stream, _)) => stream,
            Err(e) => {
                tracing::warn!(%e, "the local MCP socket's accept failed");
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                continue;
            }
        };
        // SO_PEERCRED's uid; its pid is 0 across PID namespaces, and is not judged.
        let peer = stream.peer_cred().ok().map(|cred| cred.uid());
        if peer != Some(uid) {
            tracing::warn!(?peer, "the local MCP socket refused a connection from another uid");
            continue;
        }
        let Ok(slot) = slots.clone().try_acquire_owned() else {
            tracing::warn!("the local MCP socket refused a connection: too many open");
            continue;
        };
        let app = app.clone();
        tokio::spawn(async move {
            connection(stream, app).await;
            drop(slot);
        });
    }
}

enum Line {
    Complete(Vec<u8>),
    TooLong,
    Closed,
}

/// Reads one line of at most `limit` bytes (without its newline), never buffering more than that.
async fn read_line<R: AsyncBufRead + Unpin>(reader: &mut R, limit: usize) -> std::io::Result<Line> {
    let mut line = Vec::new();
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            // A line cut by the end of input is not a message.
            return Ok(Line::Closed);
        }
        let (take, done) = match available.iter().position(|&b| b == b'\n') {
            Some(i) => (i + 1, true),
            None => (available.len(), false),
        };
        if line.len() + take > limit + 1 {
            return Ok(Line::TooLong);
        }
        line.extend_from_slice(&available[..take]);
        reader.consume(take);
        if done {
            line.pop();
            return Ok(Line::Complete(line));
        }
    }
}

/// Whether a line is an `initialize` request: its method, read without building the rest.
fn initialize(line: &[u8]) -> bool {
    #[derive(serde::Deserialize)]
    struct Method<'a> {
        #[serde(borrow)]
        method: Option<&'a str>,
    }
    serde_json::from_slice::<Method>(line).is_ok_and(|m| m.method == Some("initialize"))
}

fn encoded(value: &Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(value).expect("JSON values encode");
    bytes.push(b'\n');
    bytes
}

/// One client: its messages read in order, each served as its own task (up to [`IN_FLIGHT`] at once) and answered
/// in the order they finish. Once its input ends, the answers still due are written, then the connection closes.
async fn connection(stream: UnixStream, app: Shared) {
    let (read, mut write) = stream.into_split();
    let mut reader = BufReader::with_capacity(64 * 1024, read);
    let (answers, mut due) = mpsc::channel::<Vec<u8>>(IN_FLIGHT);
    let writer = tokio::spawn(async move {
        while let Some(answer) = due.recv().await {
            if write.write_all(&answer).await.is_err() {
                return;
            }
        }
    });
    let negotiated = Arc::new(std::sync::Mutex::new(None));
    let calls = Arc::new(Semaphore::new(IN_FLIGHT));
    loop {
        let line = match read_line(&mut reader, MAX_LINE).await {
            Ok(Line::Complete(line)) => line,
            Ok(Line::Closed) | Err(_) => break,
            Ok(Line::TooLong) => {
                let why = format!("Invalid Request: a message is longer than {MAX_LINE} bytes");
                let _ = answers.send(encoded(&mcp::error_message(None, -32600, why, None))).await;
                break;
            }
        };
        if line.trim_ascii().is_empty() {
            continue;
        }
        let inline = initialize(&line);
        let permit = calls.clone().acquire_owned().await.expect("never closed");
        let (app, answers, negotiated) = (app.clone(), answers.clone(), negotiated.clone());
        let served = async move {
            let answer = match mcp::handle(&app, Owner::Local, Transport::Stream(&negotiated), &line).await {
                // What the answer holds of the agents' budget goes once it is encoded, as over HTTP.
                Reply::Message(_, body, held) => {
                    let answer = encoded(&body);
                    drop(held);
                    Some(answer)
                }
                Reply::Accepted => None,
                Reply::Revoked => Some(encoded(&mcp::error_message(None, -32001, "the local MCP socket is off", None))),
            };
            if let Some(answer) = answer {
                let _ = answers.send(answer).await;
            }
            drop(permit);
        };
        // An `initialize` is served before the next line is read, so what follows it speaks the version it negotiated
        // even from a client that does not wait for its answer.
        if inline {
            served.await;
        } else {
            tokio::spawn(served);
        }
    }
    drop(answers);
    let _ = writer.await;
}

/// `admin mcp`: relays stdin to the socket at `path` and the socket to stdout until the admin closes the connection,
/// which it does once stdin ends and every answer due was written. Fails when the socket cannot be reached, or when the
/// admin closes the connection before stdin ends (a restart, say).
pub async fn relay(path: &Path) -> Result<(), String> {
    let stream = UnixStream::connect(path).await.map_err(|e| {
        format!(
            "{}: {e}; is the admin running, with ADMIN_LOCAL_MCP on (the default outside the demo)?",
            path.display()
        )
    })?;
    let (mut from_admin, mut to_admin) = stream.into_split();
    // Whether stdin was relayed to its end, said before the admin can see that end and close.
    let (ended, mut input) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let copied = tokio::io::copy(&mut tokio::io::stdin(), &mut to_admin).await;
        let _ = ended.send(copied.is_ok());
        let _ = to_admin.shutdown().await;
    });
    let mut stdout = tokio::io::stdout();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let n = from_admin.read(&mut buffer).await.map_err(|e| format!("reading from the admin: {e}"))?;
        if n == 0 {
            break;
        }
        stdout.write_all(&buffer[..n]).await.map_err(|e| format!("writing to stdout: {e}"))?;
        stdout.flush().await.map_err(|e| format!("writing to stdout: {e}"))?;
    }
    match input.try_recv() {
        Ok(true) => Ok(()),
        Ok(false) => Err("relaying stdin failed".into()),
        Err(_) => Err("the admin closed the connection".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio::io::AsyncBufReadExt;

    struct Client {
        lines: tokio::io::Lines<BufReader<tokio::net::unix::OwnedReadHalf>>,
        write: tokio::net::unix::OwnedWriteHalf,
    }

    impl Client {
        async fn send(&mut self, message: &Value) {
            self.write.write_all(&encoded(message)).await.unwrap();
        }

        async fn answer(&mut self) -> Option<Value> {
            let line = tokio::time::timeout(std::time::Duration::from_secs(10), self.lines.next_line()).await.unwrap();
            line.unwrap().map(|l| serde_json::from_str(&l).unwrap())
        }

        async fn ask(&mut self, message: Value) -> Value {
            self.send(&message).await;
            self.answer().await.expect("an answer")
        }
    }

    async fn served(app: &crate::app::TestApp) -> Client {
        let path = app.config.state_dir.join(SOCKET);
        let listener = bind(&path).unwrap();
        tokio::spawn(serve(listener, (*app).clone()));
        let (read, write) = UnixStream::connect(&path).await.unwrap().into_split();
        Client { lines: BufReader::new(read).lines(), write }
    }

    fn request(id: u64, method: &str, params: Value) -> Value {
        json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
    }

    fn session(app: &crate::app::TestApp, id: &str, owner: Owner) {
        let session = crate::app::Session::new(
            id.into(),
            "https://a/".into(),
            false,
            None,
            owner,
            crate::app::Features::default(),
            app.controller.clone(),
        );
        app.sessions.lock().unwrap().insert(id.into(), Arc::new(session));
    }

    #[tokio::test]
    async fn the_socket_is_private_and_a_stale_one_is_replaced_but_a_live_one_or_a_file_is_not() {
        use std::os::unix::fs::PermissionsExt;
        let dir = crate::private::test_dir("local");
        let path = dir.join(SOCKET);
        drop(std::os::unix::net::UnixListener::bind(&path).unwrap());
        let listener = bind(&path).expect("what an earlier run left is replaced");
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert!(bind(&path).unwrap_err().contains("another admin"), "a socket something answers on stays");
        drop(listener);
        std::fs::remove_file(&path).unwrap();
        std::fs::write(&path, "x").unwrap();
        assert!(bind(&path).unwrap_err().contains("not a socket"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "x", "left alone");
    }

    #[tokio::test]
    async fn a_legacy_client_negotiates_once_then_lists_and_calls_tools_on_its_own_sessions_only() {
        let app = crate::app::test_app(false);
        session(&app, "aaaaaaaaaaaa", Owner::Token(1));
        session(&app, "bbbbbbbbbbbb", Owner::Admin);
        session(&app, "cccccccccccc", Owner::Local);
        let mut c = served(&app).await;
        let before = c.ask(request(1, "tools/list", json!({}))).await;
        assert_eq!(before["error"]["code"], -32022, "nothing negotiated yet: {before}");
        let init = c.ask(request(2, "initialize", json!({"protocolVersion": "2025-06-18", "capabilities": {}}))).await;
        assert_eq!((init["id"].clone(), init["result"]["protocolVersion"].clone()), (json!(2), json!("2025-06-18")));
        c.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).await;
        let tools = c.ask(request(3, "tools/list", json!({}))).await;
        assert_eq!(tools["id"], 3, "the notification had no answer");
        assert!(tools["result"]["tools"].as_array().unwrap().len() > 4);
        let listed = c.ask(request(4, "tools/call", json!({"name": "list_sessions", "arguments": {}}))).await;
        let ids: Vec<&Value> =
            listed["result"]["structuredContent"]["sessions"].as_array().unwrap().iter().map(|s| &s["id"]).collect();
        assert_eq!(ids, [&json!("cccccccccccc")], "only a local client's session: {listed}");
        for other in ["aaaaaaaaaaaa", "bbbbbbbbbbbb"] {
            for name in ["session_status", "stop_session", "get_desktop_state"] {
                let call = request(5, "tools/call", json!({"name": name, "arguments": {"session": other}}));
                let answer = c.ask(call).await;
                assert_eq!(answer["result"]["isError"], true, "{name} on {other}: {answer}");
            }
            assert!(app.session(other).is_some(), "never stopped");
        }
        let status = c.ask(request(
            6,
            "tools/call",
            json!({"name": "session_status", "arguments": {"session": "cccccccccccc"}}),
        ));
        assert_eq!(status.await["result"]["structuredContent"]["id"], "cccccccccccc");
    }

    #[tokio::test]
    async fn a_modern_client_needs_no_headers_and_bad_lines_are_answered_as_errors() {
        let app = crate::app::test_app(false);
        let mut c = served(&app).await;
        let meta = json!({"io.modelcontextprotocol/protocolVersion": "2026-07-28",
            "io.modelcontextprotocol/clientCapabilities": {}});
        let discovered = c.ask(request(1, "server/discover", json!({"_meta": meta}))).await;
        assert_eq!(discovered["result"]["resultType"], "complete", "{discovered}");
        let call = c.ask(request(2, "tools/call", json!({"_meta": meta, "name": "list_sessions", "arguments": {}})));
        assert_eq!(call.await["result"]["structuredContent"]["sessions"], json!([]));
        c.write.write_all(b"not json\n\n").await.unwrap();
        assert_eq!(c.answer().await.unwrap()["error"]["code"], -32700, "a blank line is skipped, not answered");
        let batch = c.ask(json!([request(3, "tools/list", json!({}))])).await;
        assert_eq!(batch["error"]["code"], -32600);
    }

    #[tokio::test]
    async fn a_message_past_the_limit_is_refused_and_the_connection_closed() {
        let app = crate::app::test_app(false);
        let mut c = served(&app).await;
        // The server reads all of it: past the limit (and room for its newline) it stops.
        c.write.write_all(&vec![b'x'; MAX_LINE + 2]).await.unwrap();
        let refused = c.answer().await.unwrap();
        assert_eq!(refused["error"]["code"], -32600, "{refused}");
        assert_eq!(c.answer().await, None, "closed");
    }

    #[tokio::test]
    async fn requests_sent_right_behind_initialize_speak_the_version_it_negotiated() {
        let app = crate::app::test_app(false);
        let mut c = served(&app).await;
        let mut lines = encoded(&request(1, "initialize", json!({"protocolVersion": "2025-06-18"})));
        for id in 2..10 {
            lines.extend(encoded(&request(id, "tools/list", json!({}))));
        }
        c.write.write_all(&lines).await.unwrap();
        for _ in 1..10 {
            let answer = c.answer().await.unwrap();
            assert!(answer.get("result").is_some(), "{answer}");
        }
        assert!(initialize(br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"a":[1,{"b":"\u0041"}]}}"#));
        assert!(!initialize(br#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#) && !initialize(b"[1]"));
    }

    #[tokio::test]
    async fn answers_still_due_are_written_after_the_input_ends() {
        let app = crate::app::test_app(false);
        let mut c = served(&app).await;
        c.send(&request(1, "initialize", json!({"protocolVersion": "2025-11-25"}))).await;
        c.write.shutdown().await.unwrap();
        assert_eq!(c.answer().await.unwrap()["id"], 1);
        assert_eq!(c.answer().await, None, "then the connection closes");
    }
}
