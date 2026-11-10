//! Agent access: tokens, the agent API under `/api/agent/`, and the MCP endpoint at `/mcp`, through the running admin
//! application. The driver snapshot test needs Docker and the agent image (`AGENT_IMAGE`, default
//! `easi-run-bench/agent:dev`).

mod common;

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::Duration;

use common::{Admin, Agent, INSTANCE, TYPING as FIXTURE, docker};
use display_protocol::LeaseHolder;
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

const MODERN: &str = "2026-07-28";
/// Every token these tests create is named with this, so a run can revoke what an earlier one left.
const PREFIX: &str = "e2e agents ";

/// A fresh token named `name`, after ending every session and revoking every token an earlier run of these tests left.
async fn token(admin: &Admin, name: &str) -> Agent {
    admin.clear().await;
    admin.remove_named("tokens", |n| n.starts_with(PREFIX)).await;
    agent(admin, name).await
}

/// One more token, leaving the others.
async fn agent(admin: &Admin, name: &str) -> Agent {
    admin.token(&format!("{PREFIX}{name}")).await
}

async fn revoke(admin: &Admin, agent: &Agent) -> Value {
    admin.call(Method::DELETE, &format!("/api/tokens/{}", agent.id), None).await.expect("revoked")
}

impl Agent {
    /// A session this token starts, which `admin` counts as its own (so a failed test stops it too).
    async fn start(&self, admin: &Admin) -> String {
        let (status, session) = self.rest(Method::POST, "/api/agent/sessions", Some(json!({"url": FIXTURE}))).await;
        assert_eq!(status, StatusCode::OK, "{session}");
        let id = session["id"].as_str().expect("an id").to_owned();
        admin.adopt(&id);
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
    let seen = common::until!(60, Duration::from_millis(500), {
        let (status, windows) = agent.tool(session, "list_windows", json!({})).await;
        let shown = windows["structuredContent"]["windows"].as_array().is_some_and(|w| !w.is_empty());
        (status == StatusCode::OK && shown).then_some(())
    });
    assert!(seen.is_some(), "the agent never saw the browser window");
}

#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_token_starts_a_session_and_calls_its_desktop_tools_over_the_agent_api() {
    let admin = Admin::login().await;
    let agent = token(&admin, "rest").await;

    let (status, tools) = agent.rest(Method::GET, "/api/agent/tools", None).await;
    assert_eq!(status, StatusCode::OK);
    let names: Vec<&str> = tools["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"click") && names.contains(&"list_windows"));
    assert!(!names.iter().any(|n| n.starts_with("browser_")), "no DevTools tool in a default session");

    let session = agent.start(&admin).await;
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
    assert_eq!(described.get("saved_session"), Some(&Value::Null), "a fresh browser resumes no saved session");
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
    admin.adopt(&session);
    wait_window(&agent, &session).await;
    let windows = agent.mcp_tool(MODERN, "list_windows", json!({"session": session, "arguments": {}})).await;
    assert_eq!(windows["resultType"], "complete");
    assert!(windows["structuredContent"]["windows"].as_array().is_some_and(|w| !w.is_empty()), "{windows}");
    // A call sent again with its call_id is answered with how the first went, and not made again.
    let once = json!({"session": session, "arguments": {}, "call_id": "e2e-windows"});
    let first = agent.mcp_tool(MODERN, "list_windows", once.clone()).await;
    assert!(first["structuredContent"]["windows"].as_array().is_some_and(|w| !w.is_empty()), "{first}");
    let again = agent.mcp_tool(MODERN, "list_windows", once).await;
    assert!(again["content"][0]["text"].as_str().unwrap_or_default().contains("ended: ok"), "{again}");
    let receipt = agent.mcp_tool(MODERN, "call_status", json!({"session": session, "call_id": "e2e-windows"})).await;
    assert_eq!(receipt["structuredContent"]["state"], "ok", "{receipt}");
    let rest = |key: &'static str| {
        let request = agent.http.post(format!("{}/api/agent/sessions/{session}/tools/list_windows", agent.origin));
        request.bearer_auth(&agent.token).header("idempotency-key", key).json(&json!({})).send()
    };
    let windows: Value = rest("e2e-rest").await.unwrap().json().await.unwrap();
    assert!(windows["structuredContent"]["windows"].is_array(), "{windows}");
    let repeated: Value = rest("e2e-rest").await.unwrap().json().await.unwrap();
    assert!(repeated["content"][0]["text"].as_str().unwrap_or_default().contains("ended: ok"), "{repeated}");
    let path = format!("/api/agent/sessions/{session}/calls/e2e-rest");
    let (status, receipt) = agent.rest(Method::GET, &path, None).await;
    assert_eq!((status, &receipt["state"], &receipt["tool"]), (StatusCode::OK, &json!("ok"), &json!("list_windows")));
    let (_, described) = agent.rest(Method::GET, &format!("/api/agent/sessions/{session}"), None).await;
    let ids: Vec<&Value> = described["recent_calls"].as_array().unwrap().iter().map(|c| &c["call_id"]).collect();
    assert!(ids.contains(&&json!("e2e-windows")) && ids.contains(&&json!("e2e-rest")), "{described}");
    assert_eq!(described["running_calls"], json!([]), "{described}");

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

/// A local MCP client, as the installer adds it: `admin mcp` in the bench admin's container, one JSON-RPC message per
/// line on its stdio.
struct Local {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    id: u64,
}

impl Local {
    fn start() -> Local {
        let mut child = Command::new("docker")
            .args(["exec", "-i", common::ADMIN_CONTAINER, "admin", "mcp"])
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
    admin.adopt(&session);
    // The administrator sees it.
    let sessions = admin.get("/api/sessions").await.expect("the sessions");
    assert!(sessions.to_string().contains(&session), "{sessions}");
    let seen = common::until!(60, Duration::from_millis(500), {
        let windows = local.tool("list_windows", json!({"session": session, "arguments": {}}));
        windows["structuredContent"]["windows"].as_array().is_some_and(|w| !w.is_empty()).then_some(())
    });
    assert!(seen.is_some(), "the agent never saw the browser window");

    let stopped = local.tool("stop_session", json!({"session": session}));
    assert_eq!(stopped["isError"], false, "{stopped}");
    let listed = local.tool("list_sessions", json!({}));
    assert_eq!(listed["structuredContent"]["sessions"], json!([]));
    admin.stop(&session).await;
}

#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn an_agent_types_text_beyond_ascii_into_a_field_and_the_page_sees_it() {
    let admin = Admin::login().await;
    let agent = token(&admin, "unicode").await;
    let session = agent.start(&admin).await;
    wait_window(&agent, &session).await;
    let (_, windows) = agent.tool(&session, "list_windows", json!({})).await;
    let (pid, window) = common::first_window(windows["structuredContent"]["windows"].as_array().unwrap()).unwrap();
    // Into the fixture's field, which has the focus: text the driver's own injection cannot type.
    let call = json!({"pid": pid, "window_id": window, "text": "é—中"});
    // The agent can see the window a moment before the compositor shows it, when it is refused and nothing is typed.
    let typed = common::until!(40, Duration::from_millis(250), {
        let (status, typed) = agent.tool(&session, "type_text", call.clone()).await;
        assert_eq!(status, StatusCode::OK, "{typed}");
        (!typed.to_string().contains("unknown-root-pid")).then_some(typed)
    })
    .expect("the compositor never showed the window");
    assert_ne!(typed["isError"], true, "{typed}");
    // The driver's own answer, counting the characters the display typed.
    let said = "Typed 3 character(s) (focus-free via cua-compositor).";
    assert_eq!((&typed["content"][0]["text"], &typed["structuredContent"]["summary"]), (&json!(said), &json!(said)));
    let seen = common::until!(20, Duration::from_millis(500), {
        common::page_text(&admin, &session).await.contains("You typed: é—中").then_some(())
    });
    assert!(seen.is_some(), "the page never saw the text: {}", common::page_text(&admin, &session).await);
    // The timeline says what the agent did, never what it typed, and keeps it once the session stopped.
    let said = |timeline: &Value| {
        let events = timeline["events"].as_array().expect("its events").clone();
        // The last typing: one tried before the compositor showed the window was refused, and is there too.
        let typed = events.iter().rev().find(|e| e["kind"] == "call" && e["tool"] == "type_text").cloned();
        assert!(!timeline.to_string().contains('中'), "{timeline}");
        assert!(events.iter().any(|e| e["kind"] == "session" && e["by"] == "agent"), "{timeline}");
        typed.expect("the agent's typing")
    };
    let timeline = admin.get(&format!("/api/sessions/{session}/timeline")).await.expect("the timeline");
    let typed = said(&timeline);
    assert_eq!((&typed["outcome"], &typed["action"]), (&json!("ok"), &json!({"typed": 3})), "{typed}");
    admin.stop(&session).await;
    let runs = admin.get("/api/runs").await.expect("the stopped sessions");
    assert!(runs["runs"].as_array().unwrap().iter().any(|r| r["id"] == session), "{runs}");
    said(&admin.get(&format!("/api/sessions/{session}/timeline")).await.expect("its timeline, kept"));
    revoke(&admin, &agent).await;
}

#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_person_s_takeover_pauses_a_token_s_tools_until_they_give_back() {
    let admin = Admin::login().await;
    let agent = token(&admin, "takeover").await;
    let session = agent.start(&admin).await;
    wait_window(&agent, &session).await;

    // The agent asks for a person: its session says so, and waiting for control waits while the request is open.
    let asked = json!({"reason": "login", "message": "Sign in, then give control back"});
    let (status, requested) = agent.tool(&session, "request_person", asked).await;
    assert_eq!((status, &requested["structuredContent"]["requested"]), (StatusCode::OK, &json!(true)), "{requested}");
    let (_, described) = agent.rest(Method::GET, &format!("/api/agent/sessions/{session}"), None).await;
    assert_eq!(described["help"]["reason"], "login", "{described}");
    let (_, waited) = agent.tool(&session, "wait_for_control", json!({"timeout_s": 1})).await;
    assert_eq!(waited["structuredContent"]["control"], "waiting", "{waited}");

    // The administrator watches the token's session and takes over.
    let person = admin.watch(&session, 0).await;
    person.wait_lease(LeaseHolder::Agent).await;
    admin.take(&session, &person).await;

    // Waiting for control returns once they give it back, saying a person held it; the request is closed then.
    let (waited, ()) = tokio::join!(agent.tool(&session, "wait_for_control", json!({})), async {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        admin.hand_back(&session, &person).await;
    });
    let answer = &waited.1["structuredContent"];
    assert_eq!((&answer["control"], &answer["person_acted"]), (&json!("agent"), &json!(true)), "{answer}");
    let (_, described) = agent.rest(Method::GET, &format!("/api/agent/sessions/{session}"), None).await;
    assert_eq!(described["help"], Value::Null, "{described}");
    admin.take(&session, &person).await;

    let (status, refused) = agent.tool(&session, "list_windows", json!({})).await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    let paused = agent.mcp_tool(MODERN, "list_windows", json!({"session": session, "arguments": {}})).await;
    assert_eq!(paused["isError"], true);
    let (_, described) = agent.rest(Method::GET, &format!("/api/agent/sessions/{session}"), None).await;
    assert_eq!(described["controlled"], true);

    admin.hand_back(&session, &person).await;
    let (status, windows) = agent.tool(&session, "list_windows", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{windows}");
    // After a takeover the token's results carry no image item either.
    let (status, shot) = agent.tool(&session, "get_desktop_state", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{shot}");
    let content = shot["content"].as_array().filter(|c| !c.is_empty()).unwrap_or_else(|| panic!("no content: {shot}"));
    assert!(shot["isError"] != true && !content.iter().any(|i| i["type"] == "image"), "{shot}");
    let (_, windows) = agent.tool(&session, "list_windows", json!({})).await;
    let (pid, window) = common::first_window(windows["structuredContent"]["windows"].as_array().unwrap()).unwrap();
    // Reading the redacted tree and acting on the page go on.
    let (status, tree) = agent.tool(&session, "get_window_state", json!({"pid": pid, "window_id": window})).await;
    assert_eq!(status, StatusCode::OK, "{tree}");
    let (status, typed) =
        agent.tool(&session, "press_key", json!({"pid": pid, "window_id": window, "key": "tab"})).await;
    assert_eq!(status, StatusCode::OK, "{typed}");

    // Its status lists the latest calls, each saying who made it, so an agent can see whether one ran.
    let (_, described) = agent.rest(Method::GET, &format!("/api/agent/sessions/{session}"), None).await;
    let recent = described["recent_calls"].as_array().unwrap_or_else(|| panic!("{described}"));
    let last = recent.last().unwrap_or_else(|| panic!("{described}"));
    let name = described["recent_calls"][0]["token"].as_str().unwrap_or_default().to_owned();
    assert!(name.ends_with("takeover"), "the token's name: {described}");
    assert_eq!((&last["tool"], &last["outcome"], &last["via"]), (&json!("press_key"), &json!("ok"), &json!("token")));
    // The timeline keeps the request, its close and who made each call.
    let timeline = admin.call(Method::GET, &format!("/api/sessions/{session}/timeline"), None).await.unwrap();
    let events = timeline["events"].as_array().unwrap();
    let help: Vec<&str> = events.iter().filter(|e| e["kind"] == "help").filter_map(|e| e["state"].as_str()).collect();
    assert_eq!(help, ["asked", "done"], "{timeline}");
    assert!(events.iter().filter(|e| e["kind"] == "call").all(|e| e["via"] == "token" && e["token"] == name.as_str()));

    person.close().await;
    admin.stop(&session).await;
    revoke(&admin, &agent).await;
}

#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_token_reaches_neither_another_s_sessions_nor_anything_behind_the_login() {
    let admin = Admin::login().await;
    let owner = token(&admin, "owner").await;
    let other = agent(&admin, "other").await;
    let session = owner.start(&admin).await;

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

/// The tests that change the admin for everyone (this one finds the admin's session list empty): scripts/e2e.sh runs
/// them alone, after every other test.
mod exclusive {
    use super::*;

    #[tokio::test]
    #[ignore = "needs the bench stack: scripts/e2e.sh"]
    async fn revoking_a_token_stops_its_sessions_and_one_still_starting() {
        let admin = Admin::login().await;
        let agent = token(&admin, "revoked").await;
        let first = agent.start(&admin).await;

        // A second start is under way (the containers take seconds) when the administrator revokes the token.
        let starting = {
            let agent = agent.clone();
            tokio::spawn(
                async move { agent.rest(Method::POST, "/api/agent/sessions", Some(json!({"url": FIXTURE}))).await },
            )
        };
        // It is under way once Docker holds a resource of a session other than the first (this test runs alone): it
        // passed the token's check.
        let instance = format!("label=run.easi.instance={INSTANCE}");
        let under_way = || {
            [&["ps", "-a"][..], &["volume", "ls"], &["network", "ls"]].iter().any(|list| {
                let format = ["--filter", &instance, "--format", r#"{{.Label "run.easi.session"}}"#];
                docker(&[list, &format[..]].concat()).1.lines().any(|s| !s.is_empty() && s != first)
            })
        };
        let started = common::until!(300, Duration::from_millis(50), under_way().then_some(()));
        assert!(started.is_some(), "the second start never got under way");
        let revoked = admin.call(Method::DELETE, &format!("/api/tokens/{}", agent.id), None).await;
        // The second start counts as the login's own before anything is checked, so a failure stops it too.
        let (status, late) = starting.await.unwrap();
        if let Some(id) = late["id"].as_str() {
            admin.adopt(id);
        }
        let revoked = revoked.expect("revoked");
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
}

/// What `inspect_page` reads of a page: its structure, in a world the page cannot see, and no password's value.
#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn inspecting_a_page_reads_its_structure_unseen_and_never_a_password() {
    let admin = Admin::login().await;
    let session = admin.create("https://fixture.bench/inspect.html").await;
    admin.wait_windows(&session, "the page's window", common::first_window).await;
    let inspect = async |arguments: Value| admin.agent(&session, "inspect_page", arguments).await;

    let links = inspect(json!({"selector": "a.out"})).await.expect("the links");
    let links = &links["structuredContent"];
    assert_eq!(links["count"], 2, "{links}");
    assert_eq!(links["elements"][1]["attributes"]["href"], "https://example.test/two", "{links}");
    assert_eq!(links["title"], "Inspect", "{links}");

    let hidden = inspect(json!({"selector": "#hidden"})).await.expect("the hidden element");
    assert_eq!(hidden["structuredContent"]["elements"][0]["shown"], false, "{hidden}");

    // A password's value in neither its attributes nor its HTML, nor in a parent's.
    for selector in ["#secret", "form"] {
        let read = inspect(json!({"selector": selector, "html": true})).await.expect("the form");
        assert!(!read.to_string().contains("hunter2"), "{read}");
    }
    let name = inspect(json!({"selector": "#name"})).await.expect("the name");
    assert_eq!(name["structuredContent"]["elements"][0]["attributes"]["value"], "visible name", "{name}");

    // The page, which counts each call to its own querySelectorAll, saw none of these.
    let reads = inspect(json!({"selector": "#reads"})).await.expect("the page's count");
    assert_eq!(reads["structuredContent"]["elements"][0]["text"], "0", "{reads}");

    // At most what it is asked for, saying there is more; a selector that is none says so.
    let one = inspect(json!({"selector": "a", "limit": 1})).await.expect("one link");
    assert_eq!(
        (&one["structuredContent"]["truncated"], one["structuredContent"]["elements"].as_array().unwrap().len()),
        (&json!(true), 1)
    );
    assert!(inspect(json!({"selector": "["})).await.is_err(), "not a selector");
    // A script is refused here: this session has no Developer tools.
    let (pid, window) = admin.wait_windows(&session, "the page's window", common::first_window).await;
    let script = json!({"javascript": "1+1", "pid": pid, "window_id": window});
    assert!(admin.agent(&session, "run_javascript", script).await.is_err(), "no script without Developer tools");
    admin.stop(&session).await;
}

/// A second window takes the keyboard from the agent, which is told how to get it back: closing the window shown leaves
/// one, whose keyboard works again, and closing the last one opens the browser again.
#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn closing_the_window_shown_gives_the_agent_its_keyboard_back() {
    let admin = Admin::login().await;
    let session = admin.create(common::ARTICLE).await;
    let (pid, window) = admin.wait_windows(&session, "the page's window", common::first_window).await;
    let hotkey = |window: u64, keys: [&str; 2]| json!({"pid": pid, "window_id": window, "keys": keys});
    admin.agent(&session, "hotkey", hotkey(window, ["ctrl", "n"])).await.expect("a new window");
    let windows = |n: usize| move |windows: &[Value]| (windows.len() == n).then(|| windows.to_vec());
    let two = admin.wait_windows(&session, "a second window", windows(2)).await;

    let refused = admin.agent(&session, "hotkey", hotkey(window, ["ctrl", "w"])).await.expect("an answer");
    assert_eq!(refused["isError"], true, "{refused}");
    assert!(refused.to_string().contains("close_window"), "the agent is told the way out: {refused}");

    assert!(admin.agent(&session, "close_window", json!({"window": 1})).await.is_err(), "it takes no arguments");
    let closed = admin.agent(&session, "close_window", json!({})).await.expect("the window shown closed");
    assert_ne!(closed["isError"], true, "{closed}");
    let left = admin.wait_windows(&session, "one window left", windows(1)).await;
    assert!(two.iter().any(|w| w["window_id"] == left[0]["window_id"]), "the other window stays: {left:?}");
    let window = left[0]["window_id"].as_u64().unwrap();
    let pressed = admin.agent(&session, "hotkey", hotkey(window, ["ctrl", "t"])).await.expect("the keyboard back");
    assert_ne!(pressed["isError"], true, "{pressed}");

    admin.agent(&session, "close_window", json!({})).await.expect("the last window closed");
    let reopened = admin.wait_windows(&session, "the browser opened again", |w| {
        w.first().filter(|w| w["pid"].as_u64() != Some(pid)).map(|w| w["title"].to_string())
    });
    assert!(reopened.await.contains("Static article"), "it opens again at the session's first page");
    admin.stop(&session).await;
}

/// A page's HTML can be more than a pipe holds at once, and it still comes whole.
#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn inspecting_a_page_answers_with_more_html_than_a_pipe_holds() {
    let admin = Admin::login().await;
    let session = admin.create("https://fixture.bench/inspect.html").await;
    admin.wait_title(&session, "Inspect").await;
    for _ in 0..5 {
        let read = admin.agent(&session, "inspect_page", json!({"selector": "#large", "html": true})).await;
        let read = read.expect("the large element");
        assert_ne!(read["isError"], true, "{}", &read.to_string()[..200.min(read.to_string().len())]);
        let html = read["structuredContent"]["elements"][0]["html"].as_str().unwrap_or_default().len();
        assert!(html > 130_000, "only {html} bytes of its HTML came");
    }
    admin.stop(&session).await;
}

/// What a person pasted reaches no agent through a page's HTML, whose escapes change it, nor by being cut before the
/// redaction could recognize it.
#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn inspecting_a_page_hides_a_long_paste_and_what_html_escapes() {
    let admin = Admin::login().await;
    let session = admin.create(FIXTURE).await;
    admin.wait_title(&session, "Typing").await;
    admin.wait_windows(&session, "the page's window", common::first_window).await;
    let person = admin.watch(&session, 0).await;
    person.wait_lease(LeaseHolder::Agent).await;
    let held = admin.take(&session, &person).await;
    person.pointer(held, display_protocol::PointerKind::Move, common::FIELD.0, common::FIELD.1).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    person.click(held, common::FIELD.0, common::FIELD.1).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let paste = |text: String| person.send(display_protocol::Message::Clipboard { lease_epoch: held, text });
    // Longer than the text an element's reading keeps, with what HTML escapes.
    let long: String = (0..400).map(|i| format!("private{i:04} ")).collect();
    let special = "Meet Alice & Bob <tomorrow> ";
    paste(format!("{special}{long}")).await;
    tokio::time::sleep(Duration::from_millis(800)).await;
    admin.hand_back(&session, &person).await;
    for html in [false, true] {
        let read = admin.agent(&session, "inspect_page", json!({"selector": "#echo", "html": html})).await;
        let text = read.expect("the echo").to_string();
        for secret in ["private0000", "private0399", "Alice", "Bob"] {
            assert!(!text.contains(secret), "{secret} came through with html {html}: {}", &text[..400.min(text.len())]);
        }
        assert!(text.contains("[redacted]"), "{text}");
    }
    person.close().await;
    admin.stop(&session).await;
}

/// An agent's parallel calls each get their answer: one inspection does not end another.
#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn inspecting_a_page_twice_at_once_answers_both() {
    let admin = Admin::login().await;
    let session = admin.create("https://fixture.bench/inspect.html").await;
    admin.wait_title(&session, "Inspect").await;
    let inspect = |selector: &'static str| admin.agent(&session, "inspect_page", json!({"selector": selector}));
    for _ in 0..5 {
        let (first, second, third) = tokio::join!(inspect("a.out"), inspect("#top"), inspect("form"));
        assert_eq!(first.expect("the links")["structuredContent"]["count"], 2);
        assert_eq!(second.expect("the heading")["structuredContent"]["count"], 1);
        assert_eq!(third.expect("the form")["structuredContent"]["count"], 1);
    }
    admin.stop(&session).await;
}

/// While a person holds control an agent reads nothing of the page; what they typed never reaches it after.
#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn inspecting_a_page_waits_for_control_and_leaves_out_what_a_person_typed() {
    let admin = Admin::login().await;
    let session = admin.create(FIXTURE).await;
    admin.wait_title(&session, "Typing").await;
    admin.wait_windows(&session, "the page's window", common::first_window).await;
    let person = admin.watch(&session, 0).await;
    person.wait_lease(LeaseHolder::Agent).await;
    let held = admin.take(&session, &person).await;
    assert_eq!(admin.agent(&session, "inspect_page", json!({"selector": "#echo"})).await.expect_err("refused").0, 409);
    person.pointer(held, display_protocol::PointerKind::Move, common::FIELD.0, common::FIELD.1).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    person.click(held, common::FIELD.0, common::FIELD.1).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    person.type_keys(held, "hunter2secret").await;
    tokio::time::sleep(Duration::from_millis(800)).await;
    admin.hand_back(&session, &person).await;
    let read = admin.agent(&session, "inspect_page", json!({"selector": "#echo"})).await.expect("the echo");
    let text = read.to_string();
    assert!(!text.contains("hunter2secret") && text.contains("[redacted]"), "{text}");
    person.close().await;
    admin.stop(&session).await;
}

/// A session a person started with Developer tools runs an agent's scripts in the page's own world.
#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn developer_tools_let_an_agent_run_a_script_on_the_page() {
    let admin = Admin::login().await;
    let session = admin.create_with(json!({"url": "https://fixture.bench/inspect.html", "devtools": true})).await;
    let (pid, window) = admin.wait_windows(&session, "the page's window", common::first_window).await;
    let script = json!({"javascript": "document.title + 40 + 2", "pid": pid, "window_id": window});
    let ran = admin.agent(&session, "run_javascript", script).await.expect("the script");
    assert!(ran.to_string().contains("Inspect402"), "{ran}");
    // Reading a page is the same here.
    let read = admin.agent(&session, "inspect_page", json!({"selector": "h1"})).await.expect("the heading");
    assert_eq!(read["structuredContent"]["elements"][0]["attributes"]["id"], "top", "{read}");
    admin.stop(&session).await;
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
