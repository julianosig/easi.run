//! Agent access: tokens, the agent API under `/api/agent/`, and the MCP endpoint at `/mcp`, through the running admin
//! application. Ignored by default: scripts/e2e.sh runs it against the bench profile (see `common`); the driver
//! snapshot test needs Docker and the agent image (`AGENT_IMAGE`, default `easi-run/agent:dev`).

mod common;

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::Duration;

use common::Admin;
use display_protocol::LeaseHolder;
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

const FIXTURE: &str = "https://fixture.bench/typing.html";
const MODERN: &str = "2026-07-28";
/// Every token these tests create is named with this, so a run can revoke what an earlier one left.
const PREFIX: &str = "e2e agents ";

/// An agent holding a token: requests carry it and nothing else (no cookie, no CSRF token, no Origin).
#[derive(Clone)]
struct Agent {
    http: reqwest::Client,
    origin: String,
    token: String,
    id: String,
}

/// A fresh token named `name`, after revoking every token an earlier run of these tests left.
async fn token(admin: &Admin, name: &str) -> Agent {
    let listed = admin.get("/api/tokens").await.expect("the tokens");
    for t in listed["tokens"].as_array().into_iter().flatten() {
        if t["name"].as_str().is_some_and(|n| n.starts_with(PREFIX)) {
            let _ = admin.call(Method::DELETE, &format!("/api/tokens/{}", t["id"].as_str().unwrap()), None).await;
        }
    }
    agent(admin, name).await
}

/// One more token, leaving the others.
async fn agent(admin: &Admin, name: &str) -> Agent {
    let created = admin
        .call(Method::POST, "/api/tokens", Some(json!({"name": format!("{PREFIX}{name}")})))
        .await
        .expect("a token");
    let token = created["token"].as_str().expect("the token, once").to_owned();
    assert!(token.starts_with("easi_"));
    let listed = admin.get("/api/tokens").await.unwrap().to_string();
    assert!(!listed.contains(&token[5..]), "the token is never listed");
    Agent { http: admin.http.clone(), origin: admin.origin.clone(), token, id: created["id"].as_str().unwrap().into() }
}

async fn revoke(admin: &Admin, agent: &Agent) -> Value {
    admin.call(Method::DELETE, &format!("/api/tokens/{}", agent.id), None).await.expect("revoked")
}

impl Agent {
    async fn rest(&self, method: Method, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut request = self.http.request(method, format!("{}{path}", self.origin)).bearer_auth(&self.token);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.expect("the agent API answers");
        let status = response.status();
        (status, response.json().await.unwrap_or(Value::Null))
    }

    async fn start(&self) -> String {
        let (status, session) = self.rest(Method::POST, "/api/agent/sessions", Some(json!({"url": FIXTURE}))).await;
        assert_eq!(status, StatusCode::OK, "{session}");
        let id = session["id"].as_str().expect("an id").to_owned();
        assert_eq!(session["watch_url"], format!("{}/#session={id}", self.origin));
        id
    }

    async fn tool(&self, session: &str, tool: &str, arguments: Value) -> (StatusCode, Value) {
        self.rest(Method::POST, &format!("/api/agent/sessions/{session}/tools/{tool}"), Some(arguments)).await
    }

    /// One MCP request: modern (2026-07-28, its metadata and matching headers) or of a legacy `version`.
    async fn mcp(&self, version: &str, method: &str, mut params: Value) -> (StatusCode, Value) {
        let mut request = self
            .http
            .post(format!("{}/mcp", self.origin))
            .bearer_auth(&self.token)
            .header("accept", "application/json, text/event-stream")
            .header("mcp-protocol-version", version);
        if version == MODERN {
            params["_meta"] = json!({
                "io.modelcontextprotocol/protocolVersion": MODERN,
                "io.modelcontextprotocol/clientCapabilities": {},
                "io.modelcontextprotocol/clientInfo": {"name": "e2e", "version": "1"},
            });
            request = request.header("mcp-method", method);
            if let Some(name) = params["name"].as_str() {
                request = request.header("mcp-name", name);
            }
        }
        let response = request
            .json(&json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}))
            .send()
            .await
            .expect("the MCP endpoint answers");
        let status = response.status();
        (status, response.json().await.unwrap_or(Value::Null))
    }

    async fn mcp_tool(&self, version: &str, name: &str, arguments: Value) -> Value {
        let (status, body) = self.mcp(version, "tools/call", json!({"name": name, "arguments": arguments})).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["result"].clone()
    }
}

/// The browser window, once the agent sees it.
async fn wait_window(agent: &Agent, session: &str) {
    for _ in 0..60 {
        let (status, windows) = agent.tool(session, "list_windows", json!({})).await;
        if status == StatusCode::OK && windows["structuredContent"]["windows"].as_array().is_some_and(|w| !w.is_empty())
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    panic!("the agent never saw the browser window");
}

#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_token_starts_a_session_and_calls_its_desktop_tools_over_the_agent_api() {
    let admin = Admin::login().await;
    admin.clear().await;
    let agent = token(&admin, "rest").await;

    let (status, tools) = agent.rest(Method::GET, "/api/agent/tools", None).await;
    assert_eq!(status, StatusCode::OK);
    let names: Vec<&str> = tools["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"click") && names.contains(&"list_windows"));
    assert!(!names.iter().any(|n| n.starts_with("browser_")), "no DevTools tool in a default session");

    let session = agent.start().await;
    wait_window(&agent, &session).await;
    let (status, shot) = agent.tool(&session, "get_desktop_state", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{shot}");
    assert!(shot["content"].as_array().is_some_and(|c| c.iter().any(|i| i["type"] == "image")), "a screenshot");
    let (status, _) = agent.tool(&session, "launch_app", json!({})).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "only the pinned desktop tools");
    // run_script's longest script, which its quotes make twice as long in JSON, reaches the driver and runs.
    let script = format!("return 1;//{}", "\"".repeat(65_536 - 11));
    let (status, ran) = agent.tool(&session, "run_script", json!({"script": script})).await;
    assert_eq!(status, StatusCode::OK, "{ran}");
    assert_ne!(ran["isError"], true, "{ran}");
    let (status, described) = agent.rest(Method::GET, &format!("/api/agent/sessions/{session}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!((described["controlled"].as_bool(), described["failed"].is_null()), (Some(false), true));
    let (_, listed) = agent.rest(Method::GET, "/api/agent/sessions", None).await;
    assert_eq!(listed["sessions"].as_array().map(Vec::len), Some(1));

    // The administrator sees the token's session among theirs.
    let mine = admin.get("/api/sessions").await.unwrap();
    assert!(mine["sessions"].as_array().unwrap().iter().any(|s| s["id"] == session));

    let (status, _) = agent.rest(Method::DELETE, &format!("/api/agent/sessions/{session}"), None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = agent.rest(Method::GET, &format!("/api/agent/sessions/{session}"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    revoke(&admin, &agent).await;
}

#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn mcp_clients_of_both_eras_start_a_session_and_call_its_tools() {
    let admin = Admin::login().await;
    admin.clear().await;
    let agent = token(&admin, "mcp").await;

    let (status, discovered) = agent.mcp(MODERN, "server/discover", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{discovered}");
    assert_eq!(discovered["result"]["resultType"], "complete");
    assert_eq!(discovered["result"]["supportedVersions"][0], MODERN);
    let (_, listed) = agent.mcp(MODERN, "tools/list", json!({})).await;
    let names: Vec<&str> =
        listed["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    for name in ["start_session", "stop_session", "list_sessions", "session_status", "list_windows", "click"] {
        assert!(names.contains(&name), "{name}");
    }

    let started = agent.mcp_tool(MODERN, "start_session", json!({"url": FIXTURE})).await;
    assert_eq!(started["isError"], false, "{started}");
    let session = started["structuredContent"]["id"].as_str().expect("its id").to_owned();
    wait_window(&agent, &session).await;
    let windows = agent.mcp_tool(MODERN, "list_windows", json!({"session": session, "arguments": {}})).await;
    assert_eq!(windows["resultType"], "complete");
    assert!(windows["structuredContent"]["windows"].as_array().is_some_and(|w| !w.is_empty()), "{windows}");

    // A legacy client: initialize (no session id), then its version on every request.
    let (status, init) = agent
        .mcp(
            "2025-06-18",
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "old", "version": "1"}}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    let (_, pong) = agent.mcp("2025-06-18", "ping", json!({})).await;
    assert_eq!(pong["result"], json!({}));
    let state = agent.mcp_tool("2025-06-18", "session_status", json!({"session": session})).await;
    assert_eq!(state["structuredContent"]["controlled"], false);
    let windows = agent.mcp_tool("2025-06-18", "list_windows", json!({"session": session, "arguments": {}})).await;
    assert!(windows["structuredContent"]["windows"].as_array().is_some_and(|w| !w.is_empty()), "{windows}");

    let stopped = agent.mcp_tool(MODERN, "stop_session", json!({"session": session})).await;
    assert_eq!(stopped["isError"], false, "{stopped}");
    let listed = agent.mcp_tool(MODERN, "list_sessions", json!({})).await;
    assert_eq!(listed["structuredContent"]["sessions"], json!([]));
    revoke(&admin, &agent).await;
}

/// A local MCP client, as the installer adds it: `admin mcp` in the bench admin's container (`ADMIN_CONTAINER`, default
/// `easi-run-admin-1`), one JSON-RPC message per line on its stdio.
struct Local {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    id: u64,
}

impl Local {
    fn start() -> Local {
        let container = std::env::var("ADMIN_CONTAINER").unwrap_or_else(|_| "easi-run-admin-1".into());
        let mut child = Command::new("docker")
            .args(["exec", "-i", &container, "admin", "mcp"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("docker exec runs");
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        Local { child, input, output, id: 0 }
    }

    fn send(&mut self, message: &Value) {
        writeln!(self.input, "{message}").expect("admin mcp reads its stdin");
    }

    /// One request, and its answer: the next line, since each waits for the one before.
    fn request(&mut self, method: &str, params: Value) -> Value {
        self.id += 1;
        self.send(&json!({"jsonrpc": "2.0", "id": self.id, "method": method, "params": params}));
        let mut line = String::new();
        self.output.read_line(&mut line).expect("admin mcp answers");
        let answer: Value = serde_json::from_str(&line).unwrap_or_else(|_| panic!("one JSON-RPC line: {line:?}"));
        assert_eq!(answer["id"], self.id, "{answer}");
        answer
    }

    fn tool(&mut self, name: &str, arguments: Value) -> Value {
        self.request("tools/call", json!({"name": name, "arguments": arguments}))["result"].clone()
    }
}

impl Drop for Local {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_local_mcp_client_starts_a_session_and_calls_its_tools_over_admin_mcp() {
    let admin = Admin::login().await;
    admin.clear().await;
    let mut local = Local::start();
    let init = local.request(
        "initialize",
        json!({"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "e2e", "version": "1"}}),
    );
    assert_eq!(init["result"]["protocolVersion"], "2025-11-25", "{init}");
    local.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
    let listed = local.request("tools/list", json!({}));
    let names: Vec<&str> =
        listed["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"start_session") && names.contains(&"list_windows"), "{listed}");

    let started = local.tool("start_session", json!({"url": FIXTURE}));
    assert_eq!(started["isError"], false, "{started}");
    let session = started["structuredContent"]["id"].as_str().expect("its id").to_owned();
    // The administrator sees it.
    let sessions = admin.get("/api/sessions").await.expect("the sessions");
    assert!(sessions.to_string().contains(&session), "{sessions}");
    let mut seen = false;
    for _ in 0..60 {
        let windows = local.tool("list_windows", json!({"session": session, "arguments": {}}));
        if windows["structuredContent"]["windows"].as_array().is_some_and(|w| !w.is_empty()) {
            seen = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert!(seen, "the agent never saw the browser window");

    let stopped = local.tool("stop_session", json!({"session": session}));
    assert_eq!(stopped["isError"], false, "{stopped}");
    let listed = local.tool("list_sessions", json!({}));
    assert_eq!(listed["structuredContent"]["sessions"], json!([]));
}

#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_person_s_takeover_pauses_a_token_s_tools_until_they_give_back() {
    let admin = Admin::login().await;
    admin.clear().await;
    let agent = token(&admin, "takeover").await;
    let session = agent.start().await;
    wait_window(&agent, &session).await;

    // The administrator watches the token's session and takes over.
    let person = admin.watch(&session, 0).await;
    person.wait_lease(LeaseHolder::Agent).await;
    admin.take_control(&session, &person).await.expect("control is granted");
    person.wait_lease(LeaseHolder::You).await;

    let (status, refused) = agent.tool(&session, "list_windows", json!({})).await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    let paused = agent.mcp_tool(MODERN, "list_windows", json!({"session": session, "arguments": {}})).await;
    assert_eq!(paused["isError"], true);
    let (_, described) = agent.rest(Method::GET, &format!("/api/agent/sessions/{session}"), None).await;
    assert_eq!(described["controlled"], true);

    admin.give_back(&session, &person).await.expect("control is given back");
    person.wait_lease(LeaseHolder::Agent).await;
    let (status, windows) = agent.tool(&session, "list_windows", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{windows}");
    // After a takeover the token's results carry no image item either.
    let (_, shot) = agent.tool(&session, "get_desktop_state", json!({})).await;
    assert!(!shot["content"].as_array().is_some_and(|c| c.iter().any(|i| i["type"] == "image")), "{shot}");
    let (pid, window) = {
        let (_, windows) = agent.tool(&session, "list_windows", json!({})).await;
        let w = &windows["structuredContent"]["windows"][0];
        (w["pid"].as_u64().unwrap(), w["window_id"].as_u64().unwrap())
    };
    // Reading the redacted tree and acting on the page go on.
    let (status, tree) = agent.tool(&session, "get_window_state", json!({"pid": pid, "window_id": window})).await;
    assert_eq!(status, StatusCode::OK, "{tree}");
    let (status, typed) =
        agent.tool(&session, "press_key", json!({"pid": pid, "window_id": window, "key": "tab"})).await;
    assert_eq!(status, StatusCode::OK, "{typed}");

    person.close().await;
    admin.stop(&session).await;
    revoke(&admin, &agent).await;
}

#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_token_reaches_neither_another_s_sessions_nor_anything_behind_the_login() {
    let admin = Admin::login().await;
    admin.clear().await;
    let owner = token(&admin, "owner").await;
    let other = agent(&admin, "other").await;
    let session = owner.start().await;

    let path = format!("/api/agent/sessions/{session}");
    assert_eq!(other.rest(Method::GET, &path, None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(other.tool(&session, "list_windows", json!({})).await.0, StatusCode::NOT_FOUND);
    assert_eq!(other.rest(Method::DELETE, &path, None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(other.rest(Method::GET, "/api/agent/sessions", None).await.1["sessions"], json!([]));
    let status = other.mcp_tool(MODERN, "session_status", json!({"session": session})).await;
    assert_eq!(status["isError"], true);
    let stop = other.mcp_tool(MODERN, "stop_session", json!({"session": session})).await;
    assert_eq!(stop["isError"], true);
    assert_eq!(owner.rest(Method::GET, &path, None).await.0, StatusCode::OK, "still running, still its owner's");

    // Everything behind the administrator's login refuses a token.
    // Each with a well-formed body, so the login is what refuses it.
    let viewer = json!({"viewer": "0".repeat(32)});
    let cookie_only = [
        (Method::GET, "/api/sessions".to_owned(), None),
        (Method::POST, "/api/sessions".to_owned(), Some(json!({"url": FIXTURE}))),
        (Method::GET, "/api/tokens".to_owned(), None),
        (Method::POST, "/api/tokens".to_owned(), Some(json!({"name": format!("{PREFIX}minted")}))),
        (Method::DELETE, format!("/api/tokens/{}", other.id), None),
        (Method::GET, format!("/api/sessions/{session}/stats"), None),
        (Method::POST, format!("/api/sessions/{session}/agent"), Some(json!({"tool": "list_windows"}))),
        (Method::POST, format!("/api/sessions/{session}/control"), Some(viewer.clone())),
        (Method::POST, format!("/api/sessions/{session}/release"), Some(viewer)),
        (
            Method::PUT,
            format!("/api/sessions/{session}/settings"),
            Some(json!({"codec": "auto", "bitrate_kbps": 4000, "max_fps": 30})),
        ),
        (Method::DELETE, format!("/api/sessions/{session}"), None),
    ];
    for (method, path, body) in cookie_only {
        let (status, _) = owner.rest(method.clone(), &path, body).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {path}");
    }
    let upgrade = owner
        .http
        .get(format!("{}/ws/sessions/{session}?viewer={}", owner.origin, "0".repeat(32)))
        .bearer_auth(&owner.token)
        .header("origin", &owner.origin)
        .header("connection", "Upgrade")
        .header("upgrade", "websocket")
        .header("sec-websocket-version", "13")
        .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
        .send()
        .await
        .unwrap();
    assert_eq!(upgrade.status(), StatusCode::UNAUTHORIZED, "no viewer stream for a token");
    // And a page on another site cannot use one.
    let elsewhere = owner
        .http
        .get(format!("{}/api/agent/sessions", owner.origin))
        .bearer_auth(&owner.token)
        .header("origin", "https://evil.example")
        .send()
        .await
        .unwrap();
    assert_eq!(elsewhere.status(), StatusCode::FORBIDDEN);

    admin.stop(&session).await;
    revoke(&admin, &owner).await;
    revoke(&admin, &other).await;
}

#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn revoking_a_token_stops_its_sessions_and_one_still_starting() {
    let admin = Admin::login().await;
    admin.clear().await;
    let agent = token(&admin, "revoked").await;
    let first = agent.start().await;

    // A second start is under way (the containers take seconds) when the administrator revokes the token.
    let starting = {
        let agent = agent.clone();
        tokio::spawn(
            async move { agent.rest(Method::POST, "/api/agent/sessions", Some(json!({"url": FIXTURE}))).await },
        )
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    let revoked = revoke(&admin, &agent).await;
    let (status, late) = starting.await.unwrap();
    match status {
        // The usual case: the start was still under way, and its session was stopped instead of listed.
        StatusCode::UNAUTHORIZED => assert_eq!(revoked["stopped"], 1, "{revoked}"),
        // It was listed before the revocation, which then stopped it too.
        StatusCode::OK => assert_eq!(revoked["stopped"], 2, "{revoked} {late}"),
        other => panic!("the start under way answered {other}: {late}"),
    }
    let left = admin.get("/api/sessions").await.unwrap();
    assert_eq!(left["sessions"], json!([]), "every session of the token is gone");
    assert!(admin.get(&format!("/api/sessions/{first}/stats")).await.is_err());

    // Refused like any unknown token.
    let (status, _) = agent.rest(Method::GET, "/api/agent/sessions", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "the token is refused");
    let (status, _) = agent.mcp(MODERN, "server/discover", json!({})).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

/// The pinned tool snapshot (`admin/src/tools.json`) is what the agent image's driver lists today for those tools:
/// checked by the script that writes it, through the same handshake.
#[tokio::test]
#[ignore = "needs Docker and the agent image: scripts/e2e.sh"]
async fn the_pinned_tools_are_the_live_driver_s() {
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/../scripts/tools.py");
    let out = std::process::Command::new("python3").args([script, "--check"]).output().expect("python3 runs");
    assert!(out.status.success(), "scripts/tools.py --check: {}", String::from_utf8_lossy(&out.stderr));
}
