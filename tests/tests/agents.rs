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
/// Every token these tests create is named with this and a name of its test's own, so a test can revoke what an
/// earlier run of it left, and leaves alone the tokens of the tests that run beside it.
const PREFIX: &str = "e2e agents ";

/// A fresh token named `name`, after ending this login's sessions.
async fn token(admin: &Admin, name: &str) -> Agent {
    admin.clear().await;
    agent(admin, name).await
}

/// A fresh token named `name`, after revoking the one an earlier run left under that name.
async fn agent(admin: &Admin, name: &str) -> Agent {
    let named = format!("{PREFIX}{name}");
    admin.remove_named("tokens", |n| n == named).await;
    admin.token(&named).await
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
/// The browser's window as the token's agent lists it, once it is shown: its pid and id. (The driver may list a window
/// before the display has mapped it, with no size yet: that one is not waited on.)
async fn wait_window(agent: &Agent, session: &str) -> (u64, u64) {
    let seen = common::until!(60, Duration::from_millis(500), {
        let (status, windows) = agent.tool(session, "list_windows", json!({})).await;
        let shown = windows["structuredContent"]["windows"].as_array().and_then(|w| common::first_window(w));
        shown.filter(|_| status == StatusCode::OK)
    });
    seen.expect("the agent never saw the browser window")
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
    let (pid, window) = wait_window(&agent, &session).await;
    // A pause between characters, which nothing keeps into a window: refused rather than typed at once.
    let paced = json!({"pid": pid, "window_id": window, "text": "abc", "delay_ms": 200});
    let (status, paced) = agent.tool(&session, "type_text", paced).await;
    assert_eq!(status, StatusCode::OK, "{paced}");
    assert_eq!(
        (&paced["isError"], &paced["structuredContent"]["code"]),
        (&json!(true), &json!("delay_unsupported")),
        "{paced}"
    );
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
    // After a takeover the token sees the screen again.
    let (status, shot) = agent.tool(&session, "get_desktop_state", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{shot}");
    let content = shot["content"].as_array().filter(|c| !c.is_empty()).unwrap_or_else(|| panic!("no content: {shot}"));
    assert!(shot["isError"] != true && content.iter().any(|i| i["type"] == "image"), "{shot}");
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
    // The page itself, loaded: a reading asked of a browser whose page is not there yet says the page did not answer,
    // as it did here once beside the other suites (and, asked as soon as the window was listed, in three runs of
    // three with eight busy loops on the browser's processors, where waiting for the page passed three of three).
    admin.wait_title(&session, "Inspect").await;
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

/// `navigate` takes the page shown to an address and answers once it loaded, with where the page is then and, when
/// asked, what `inspect_page` reads there, in the one call, a password's value never in it. What it did not reach it does not read, and says what became of the navigation instead.
#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn an_agent_goes_to_an_address_and_reads_the_page_there_in_one_call() {
    let admin = Admin::login().await;
    let session = admin.create("about:blank").await;
    admin.wait_windows(&session, "the empty page's window", common::first_window).await;
    let go = async |arguments: Value| admin.agent(&session, "navigate", arguments).await;
    let page = "https://fixture.bench/inspect.html";

    // Loaded, and read there: the links, with where the page is.
    let read = go(json!({"url": page, "read": {"selector": "a.out"}})).await.expect("the page, read");
    let said = &read["structuredContent"];
    let there = json!({"requested": page, "outcome": "loaded", "url": page, "title": "Inspect", "status": 200});
    assert_eq!((&said["navigation"], &read["isError"]), (&there, &json!(false)), "{read}");
    assert_eq!(
        (&said["count"], &said["elements"][1]["attributes"]["href"]),
        (&json!(2), &json!("https://example.test/two"))
    );
    // A password field's value is never in what is read there.
    let inputs = go(json!({"url": page, "read": {"selector": "form input"}})).await.expect("its fields");
    assert_eq!(inputs["structuredContent"]["count"], 2, "{inputs}");
    assert!(!inputs.to_string().contains("hunter2"), "{inputs}");
    // With nothing to read asked for, only where it is; and the page saw none of the readings in its own world.
    let plain = go(json!({"url": format!("{page}?again")})).await.expect("the page");
    let keys: Vec<&String> = plain["structuredContent"].as_object().expect("an object").keys().collect();
    assert_eq!(keys, ["navigation"], "{plain}");
    let reads = admin.agent(&session, "inspect_page", json!({"selector": "#reads"})).await.expect("the page's count");
    assert_eq!(reads["structuredContent"]["elements"][0]["text"], "0", "{reads}");

    // It got there, and its page was not read as asked (a selector that is none): an error that keeps where it got.
    let unread = go(json!({"url": page, "read": {"selector": "["}})).await.expect("an answer");
    let said = &unread["structuredContent"];
    assert_eq!((&unread["isError"], &said["navigation"]["outcome"]), (&json!(true), &json!("loaded")), "{unread}");
    assert_eq!(said["read_error"], "not a valid CSS selector", "{unread}");

    // A redirect is followed to where it ends, and an address of the document shown loads nothing.
    let moved = go(json!({"url": "https://fixture.bench/redirect?to=/inspect.html"})).await.expect("redirected");
    let moved = &moved["structuredContent"]["navigation"];
    assert_eq!((&moved["outcome"], &moved["url"]), (&json!("loaded"), &json!(page)), "{moved}");
    let same = go(json!({"url": format!("{page}#top"), "read": {"selector": "h1"}})).await.expect("the same document");
    let said = &same["structuredContent"];
    assert_eq!(
        (&said["navigation"]["outcome"], &said["elements"][0]["text"]),
        (&json!("same_document"), &json!("A page to read"))
    );
    // A page the server has not, said with a page, is a page all the same, with its status; said with nothing, the
    // browser shows its own error, and the navigation failed.
    let missing = go(json!({"url": "https://fixture.bench/missing"})).await.expect("an answer");
    let missing = &missing["structuredContent"]["navigation"];
    assert_eq!(
        (&missing["outcome"], &missing["status"], &missing["title"]),
        (&json!("loaded"), &json!(404), &json!("Missing"))
    );
    let bare = go(json!({"url": "https://fixture.bench/no-such-page"})).await.expect("an answer");
    let said = &bare["structuredContent"]["navigation"];
    assert_eq!((&bare["isError"], &said["outcome"]), (&json!(true), &json!("failed")), "{bare}");
    assert_eq!(said["error"], "net::ERR_HTTP_RESPONSE_CODE_FAILURE", "{bare}");

    // Nothing answers there: an error result with the browser's word, and nothing read as if it were the page.
    let nowhere =
        go(json!({"url": "https://no-such-host.invalid/", "read": {"selector": "a"}})).await.expect("an answer");
    let said = &nowhere["structuredContent"];
    assert_eq!((&nowhere["isError"], &said["navigation"]["outcome"]), (&json!(true), &json!("failed")), "{nowhere}");
    assert!(said["navigation"]["error"].as_str().is_some_and(|error| error.starts_with("net::")), "{nowhere}");
    assert_eq!(said["navigation"]["began"], false, "{nowhere}");
    assert!(said.get("elements").is_none() && said["navigation"].get("url").is_none(), "{nowhere}");
    // A file is downloaded, and the page stays where it was: said, not read.
    go(json!({"url": page})).await.expect("the page again");
    let file =
        go(json!({"url": "https://fixture.bench/big?size=4096", "read": {"selector": "a"}})).await.expect("an answer");
    let said = &file["structuredContent"];
    assert_eq!((&file["isError"], &said["navigation"]["outcome"]), (&json!(false), &json!("download")), "{file}");
    assert!(said.get("elements").is_none(), "{file}");

    // Only the addresses a session may start at, and only the arguments it has.
    for refused in [
        json!({"url": "file:///etc/passwd"}),
        json!({"url": "javascript:alert(1)"}),
        json!({"url": "chrome://settings"}),
        json!({"url": "view-source:https://fixture.bench/inspect.html"}),
        json!({"url": page, "read": {"selector": "a", "script": "1"}}),
        json!({}),
    ] {
        assert!(go(refused.clone()).await.is_err(), "{refused} was taken");
    }
    let still = admin.agent(&session, "inspect_page", json!({"selector": "title"})).await.expect("the page, still");
    assert_eq!(still["structuredContent"]["title"], "Inspect", "{still}");
    admin.stop(&session).await;
}

/// An agent's token gets `navigate` as it gets `inspect_page`: listed with its arguments, and called over the agent
/// API on its own sessions only.
#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_token_s_agent_navigates_its_own_session() {
    let admin = Admin::login().await;
    let agent = agent(&admin, "navigator").await;
    let (status, tools) = agent.rest(reqwest::Method::GET, "/api/agent/tools", None).await;
    assert_eq!(status, StatusCode::OK, "{tools}");
    let listed = tools["tools"].as_array().expect("the tools").iter().find(|tool| tool["name"] == "navigate");
    let listed = listed.unwrap_or_else(|| panic!("navigate is not listed: {tools}"));
    assert_eq!(listed["inputSchema"]["required"], json!(["url"]), "{listed}");
    assert_eq!(listed["annotations"]["readOnlyHint"], false, "{listed}");
    let (status, started) =
        agent.rest(reqwest::Method::POST, "/api/agent/sessions", Some(json!({"url": "about:blank"}))).await;
    assert_eq!(status, StatusCode::OK, "{started}");
    let session = started["id"].as_str().expect("its id").to_owned();
    wait_window(&agent, &session).await;
    let arguments = json!({"url": "https://fixture.bench/inspect.html", "read": {"selector": "h1"}});
    let (status, read) = agent.tool(&session, "navigate", arguments.clone()).await;
    assert_eq!(status, StatusCode::OK, "{read}");
    let said = &read["structuredContent"];
    assert_eq!(
        (&said["navigation"]["outcome"], &said["elements"][0]["text"]),
        (&json!("loaded"), &json!("A page to read"))
    );
    // Over MCP, with a `call_id`: sent again (its answer lost), it is answered with how the first went, and the
    // page is not taken there twice.
    let once =
        json!({"session": session, "arguments": {"url": "https://fixture.bench/article.html"}, "call_id": "go-1"});
    let went = agent.mcp_tool(MODERN, "navigate", once.clone()).await;
    assert_eq!(went["structuredContent"]["navigation"]["outcome"], "loaded", "{went}");
    let again = agent.mcp_tool(MODERN, "navigate", once).await;
    assert!(again["content"][0]["text"].as_str().unwrap_or_default().contains("ended: ok"), "{again}");
    let receipt = agent.mcp_tool(MODERN, "call_status", json!({"session": session, "call_id": "go-1"})).await;
    let receipt = &receipt["structuredContent"];
    assert_eq!((&receipt["state"], &receipt["tool"]), (&json!("ok"), &json!("navigate")), "{receipt}");
    let (_, described) = agent.rest(Method::GET, &format!("/api/agent/sessions/{session}"), None).await;
    let made = described["recent_calls"].as_array().unwrap().iter().filter(|call| call["call_id"] == "go-1").count();
    assert_eq!((made, &described["running_calls"]), (1, &json!([])), "one navigation, recorded once: {described}");
    // Another token's agent does not reach it.
    let other = token(&admin, "stranger").await;
    let (status, _) = other.tool(&session, "navigate", arguments).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    revoke(&admin, &other).await;
    revoke(&admin, &agent).await;
}

/// A navigation asked the moment its session exists, before its browser has a page, waits for one: it is not
/// answered that the browser cannot navigate, nor that nothing answered.
#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_navigation_asked_as_its_session_starts_waits_for_the_browser_s_page() {
    let admin = Admin::login().await;
    let session = admin.create("about:blank").await;
    let went = admin
        .agent(&session, "navigate", json!({"url": "https://fixture.bench/inspect.html", "read": {"selector": "h1"}}))
        .await
        .expect("an answer");
    let said = &went["structuredContent"];
    assert_eq!(
        (&said["navigation"]["outcome"], &said["elements"][0]["text"]),
        (&json!("loaded"), &json!("A page to read")),
        "{went}"
    );
    admin.stop(&session).await;
}

/// A page whose document never finishes loading: the navigation is answered as not loaded in time, an error result
/// that says the page began, and the session goes on (the page is read as far as it came).
#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_page_that_never_finishes_loading_is_answered_as_not_loaded_in_time() {
    let admin = Admin::login().await;
    let session = admin.create("about:blank").await;
    admin.wait_windows(&session, "the empty page's window", common::first_window).await;
    let began = std::time::Instant::now();
    let hung = admin
        .agent(&session, "navigate", json!({"url": "https://fixture.bench/hang", "read": {"selector": "p"}}))
        .await;
    let hung = hung.expect("an answer");
    assert!(began.elapsed() < Duration::from_secs(55), "answered after {:?}", began.elapsed());
    let said = &hung["structuredContent"];
    assert_eq!((&hung["isError"], &said["navigation"]["outcome"]), (&json!(true), &json!("timeout")), "{hung}");
    assert_eq!(said["navigation"]["began"], true, "its document began: {hung}");
    assert!(said.get("elements").is_none(), "{hung}");
    let read = admin.agent(&session, "inspect_page", json!({"selector": "#began"})).await.expect("what came of it");
    assert_eq!(read["structuredContent"]["elements"][0]["text"], "began", "{read}");
    let next =
        admin.agent(&session, "navigate", json!({"url": "https://fixture.bench/inspect.html"})).await.expect("on");
    assert_eq!(next["structuredContent"]["navigation"]["outcome"], "loaded", "{next}");
    admin.stop(&session).await;
}

/// A person who takes control while a navigation waits for its page gets it at once: the call answers that a person
/// holds control, no navigation begins while they do, and the agent navigates again once they give control back.
#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_takeover_gives_up_a_navigation_that_waits_and_none_begins_while_a_person_holds_control() {
    let admin = Admin::login().await;
    let session = admin.create("https://fixture.bench/inspect.html").await;
    admin.wait_title(&session, "Inspect").await;
    let person = admin.watch(&session, 0).await;
    person.wait_lease(LeaseHolder::Agent).await;
    let waiting = admin.agent(&session, "navigate", json!({"url": "https://fixture.bench/hang"}));
    let taking = async {
        tokio::time::sleep(Duration::from_secs(2)).await;
        let began = std::time::Instant::now();
        let held = admin.take(&session, &person).await;
        (held, began.elapsed())
    };
    let (went, (_held, took)) = tokio::join!(waiting, taking);
    assert!(took < Duration::from_secs(4), "control came after {took:?}");
    assert_eq!(went.expect_err("given up for the person").0, 409);
    let page = json!({"url": "https://fixture.bench/inspect.html"});
    assert_eq!(admin.agent(&session, "navigate", page.clone()).await.expect_err("refused").0, 409);
    // What the person sees is what the navigation had begun, and nothing the refused one asked for.
    tokio::time::sleep(Duration::from_secs(1)).await;
    admin.hand_back(&session, &person).await;
    let title = admin.agent(&session, "inspect_page", json!({"selector": "title"})).await.expect("the page");
    assert_eq!(title["structuredContent"]["title"], "Hang", "{title}");
    let after = admin.agent(&session, "navigate", page).await.expect("a navigation again");
    assert_eq!(after["structuredContent"]["navigation"]["outcome"], "loaded", "{after}");
    person.close().await;
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
    let windows = |n: usize| move |windows: &[Value]| (windows.len() == n).then(|| windows.to_vec());
    // The agent can see the window a moment before the compositor shows it, when its keys are refused: tried again.
    let opened = common::until!(40, Duration::from_millis(250), {
        let pressed = admin.agent(&session, "hotkey", hotkey(window, ["ctrl", "n"])).await.expect("an answer");
        (pressed["isError"] != true).then_some(())
    });
    assert!(opened.is_some(), "the keyboard never reached the first window");
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
    // Another Chrome, once its page has loaded (its window is listed a moment before it has a title).
    let reopened = |w: &[Value]| {
        let other = w.first().filter(|w| w["pid"].as_u64().is_some_and(|new| new != pid))?;
        other["title"].as_str().is_some_and(|title| title.contains("Static article")).then_some(())
    };
    admin.wait_windows(&session, "the browser opened again at the session's first page", reopened).await;
    admin.stop(&session).await;
}

/// A session keeps at most `BROWSER_MAX_TABS` tabs (8 here, the default): once its browser has restored its own, a new
/// tab past them closes as it opens, the agent's next result says why, and the timeline records it.
#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_new_tab_past_the_limit_closes_as_it_opens_and_the_agent_is_told() {
    let admin = Admin::login().await;
    let session = admin.create(common::ARTICLE).await;
    let (pid, window) = admin.wait_windows(&session, "the page's window", common::first_window).await;
    // Past the time the browser has to restore a saved session's tabs, which are never refused.
    tokio::time::sleep(Duration::from_secs(21)).await;
    let new_tab = json!({"pid": pid, "window_id": window, "keys": ["ctrl", "t"]});
    for _ in 0..7 {
        admin.agent(&session, "hotkey", new_tab.clone()).await.expect("a new tab");
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    let past = admin.agent(&session, "hotkey", new_tab.clone()).await.expect("one tab past the limit");
    let note = "keeps at most 8 browser tabs";
    // Told at the next result: the press's own, if the refusal came first, or a later one.
    let told = past.to_string().contains(note)
        || common::until!(20, Duration::from_millis(500), {
            let read = admin.agent(&session, "list_windows", json!({})).await.expect("the windows");
            read.to_string().contains(note).then_some(())
        })
        .is_some();
    assert!(told, "the agent was never told the tab closed");
    let timeline = admin.get(&format!("/api/sessions/{session}/timeline")).await.expect("the timeline");
    let events = timeline["events"].as_array().expect("its events");
    assert!(events.iter().any(|e| e["kind"] == "tab" && e["refused"] == 8), "{timeline}");
    // Told once.
    let again = admin.agent(&session, "list_windows", json!({})).await.expect("the windows");
    assert!(!again.to_string().contains(note), "{again}");
    admin.stop(&session).await;
}

/// A page that uses the browser's memory up loses its process to the kernel, alone. The agent's next result says a
/// tab's page crashed and why, and the timeline records it; the window keeps its process and its number in the agent's
/// list (Chrome names a crashed tab's window "Crashed" to assistive technology, by which the driver finds a window's
/// process); and the tab loads a page again.
#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_page_that_uses_the_memory_up_is_told_to_the_agent_and_its_tab_goes_on() {
    let admin = Admin::login().await;
    let session = admin.create("https://fixture.bench/memory.html").await;
    admin.wait_title(&session, "Memory").await;
    let (pid, window) = admin.wait_windows(&session, "the page's window", common::first_window).await;
    let target = json!({"pid": pid, "window_id": window});
    admin.agent(&session, "get_window_state", target.clone()).await.expect("the window read");
    let click = json!({"pid": pid, "window_id": window, "x": 300, "y": 300});
    let clicked = admin.agent(&session, "click", click).await.expect("a click on the page");
    assert_ne!(clicked["isError"], true, "{clicked}");

    // Told at the next result: the click's own, if the page was gone by then, or a later one.
    let note = "reached this session's memory limit";
    let told = clicked.to_string().contains(note)
        || common::until!(120, Duration::from_millis(500), {
            let read = admin.agent(&session, "list_windows", json!({})).await.expect("the windows");
            read.to_string().contains(note).then_some(())
        })
        .is_some();
    assert!(told, "the agent was never told its page crashed");
    // The window as it was listed before, which the driver's tools still take.
    let read = admin.agent(&session, "list_windows", json!({})).await.expect("the windows");
    let listed = common::first_window(read["structuredContent"]["windows"].as_array().expect("its windows"));
    assert_eq!(listed, Some((pid, window)), "{read}");
    let state = admin.agent(&session, "get_window_state", target.clone()).await.expect("the crashed tab's window read");
    assert_ne!(state["isError"], true, "{state}");
    let timeline = admin.get(&format!("/api/sessions/{session}/timeline")).await.expect("the timeline");
    let events = timeline["events"].as_array().expect("its events");
    assert!(events.iter().any(|e| e["kind"] == "tab" && e["crashed"] == "memory"), "{timeline}");
    // Told once.
    let again = admin.agent(&session, "list_windows", json!({})).await.expect("the windows");
    assert!(!again.to_string().contains(note), "{again}");

    // The tab goes on: its address bar takes another address.
    let keys = json!({"pid": pid, "window_id": window, "keys": ["ctrl", "l"]});
    admin.agent(&session, "hotkey", keys).await.expect("the address bar");
    let address = json!({"pid": pid, "window_id": window, "text": format!("{}\n", common::ARTICLE)});
    admin.agent(&session, "type_text", address).await.expect("an address typed");
    admin.wait_title(&session, "Static article").await;
    admin.stop(&session).await;
}

/// The keys a call sent, as two calls or one batch of the same two.
async fn shortcut_then_text(admin: &Admin, session: &str, batch: bool, keys: Value, text: Value) {
    if batch {
        let steps = json!({"steps": [{"tool": "hotkey", "args": keys}, {"tool": "type_text", "args": text}]});
        // The agent can see the window a moment before the compositor shows it, when its keys are refused.
        let ran = common::until!(40, Duration::from_millis(250), {
            let ran = admin.agent(session, "run_steps", steps.clone()).await.expect("an answer");
            (ran["isError"] != true).then_some(())
        });
        assert!(ran.is_some(), "the batch never ran");
    } else {
        let pressed = common::until!(40, Duration::from_millis(250), {
            let pressed = admin.agent(session, "hotkey", keys.clone()).await.expect("an answer");
            (pressed["isError"] != true).then_some(())
        });
        assert!(pressed.is_some(), "the keyboard never reached the window");
        admin.agent(session, "type_text", text).await.expect("an address typed");
    }
}

/// Whether the session's window comes to show the static article.
async fn went_to_the_article(admin: &Admin, session: &str) -> bool {
    let went = common::until!(20, Duration::from_millis(250), {
        let listed = admin.agent(session, "list_windows", json!({})).await.unwrap_or_default();
        let title = listed["structuredContent"]["windows"][0]["title"].as_str().unwrap_or_default();
        title.starts_with("Static article").then_some(())
    });
    went.is_some()
}

/// An address typed right after the shortcut that focuses the address bar is gone to, as two calls or as one batch,
/// whatever the page takes to leave the shortcut to the browser: here 400 ms, every time, with the keyboard in the
/// page or in a sandboxed frame of it, and in a browser that has just started, whose page answers its first key slowly. Chrome moves the focus only once the page has answered the
/// shortcut's key, and text that reaches it before that goes to the page and is lost: the shortcut's line is answered,
/// and the text applied, only once the browser has handled it.
#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn an_address_typed_right_after_its_shortcut_is_gone_to_however_long_the_page_takes() {
    let admin = Admin::login().await;
    let pages = ["keys.html?slow=400", "keys.html?slow=400", "keys.html?slow=400&frame", "cookie.html", "cookie.html"];
    for (round, page) in pages.iter().enumerate() {
        let session = admin.create(&format!("https://fixture.bench/{page}")).await;
        if page.ends_with("frame") {
            // The keyboard is in the page's frame once it loaded.
            admin.wait_title(&session, "Keys framed").await;
        }
        let (pid, window) = admin.wait_windows(&session, "the page's window", common::first_window).await;
        let keys = json!({"pid": pid, "window_id": window, "keys": ["ctrl", "l"]});
        let text = json!({"pid": pid, "window_id": window, "text": format!("{}\n", common::ARTICLE)});
        shortcut_then_text(&admin, &session, round % 2 == 1, keys, text).await;
        assert!(went_to_the_article(&admin, &session).await, "{page}, round {round}: the address was not gone to");
        admin.stop(&session).await;
    }
}

/// What tells the display that the browser has handled an agent's keys is nothing a page hears: it hears the keys
/// typed, each once, and when a shortcut takes the keyboard to the address bar, that it lost the keyboard, once, and
/// never that it got it back.
#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_page_hears_the_keys_an_agent_types_and_nothing_else() {
    let admin = Admin::login().await;
    let session = admin.create("https://fixture.bench/keys.html").await;
    admin.wait_title(&session, "Keys").await;
    let (pid, window) = admin.wait_windows(&session, "the page's window", common::first_window).await;
    let typed = common::until!(40, Duration::from_millis(250), {
        let text = json!({"pid": pid, "window_id": window, "text": "ab"});
        let typed = admin.agent(&session, "type_text", text).await.expect("an answer");
        (typed["isError"] != true).then_some(())
    });
    assert!(typed.is_some(), "the keyboard never reached the window");
    let key = json!({"pid": pid, "window_id": window, "key": "Escape"});
    admin.agent(&session, "press_key", key).await.expect("a key pressed");
    let keys = json!({"pid": pid, "window_id": window, "keys": ["ctrl", "l"]});
    admin.agent(&session, "hotkey", keys).await.expect("a shortcut pressed");
    // What the page heard since it loaded with the keyboard (told so, more than once at times).
    let heard = |text: &str| -> String {
        let said = text.lines().find_map(|line| line.trim().strip_prefix("- static = \"heard:"));
        let said =
            said.and_then(|rest| rest.strip_suffix('"')).unwrap_or_else(|| panic!("the page says nothing: {text}"));
        said.trim_start_matches(" focus:").to_owned()
    };
    // The keys typed, each once; then the shortcut's, until the page left it to the browser, which took the keyboard:
    // its releases reach the page or not, as the browser was quick or not.
    let typed = " keydown:a keypress:a keyup:a keydown:b keypress:b keyup:b keydown:Escape keyup:Escape";
    let all = |released: &str| format!("{typed} keydown:Control keydown:l{released} blur:");
    let expected = [all(""), all(" keyup:l"), all(" keyup:l keyup:Control")];
    let lost = common::until!(20, Duration::from_millis(250), {
        expected.contains(&heard(&common::page_text(&admin, &session).await)).then_some(())
    });
    assert!(lost.is_some(), "the page heard something else: {}", heard(&common::page_text(&admin, &session).await));
    // And nothing after that: never that it got the keyboard back.
    tokio::time::sleep(Duration::from_secs(1)).await;
    let after = heard(&common::page_text(&admin, &session).await);
    assert!(expected.contains(&after), "the page heard more: {after}");
    admin.stop(&session).await;
}

/// Chrome also names a window "Network error" to assistive technology while its tab shows the browser's own error page
/// (a host that does not answer, a site that is down). A session whose first listing came then had no window
/// remembered to give back: the driver is asked for the browser's process's windows, which it lists by their
/// accessibility, and the agent gets a window its tools take.
#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_window_first_listed_on_an_error_page_is_one_the_tools_take() {
    let admin = Admin::login().await;
    let session = admin.create("https://no-such-host.invalid/").await;
    // The error page, seen without listing a window: the launcher reads the page for inspect_page, not the driver.
    let failed = common::until!(120, Duration::from_millis(500), {
        let read = admin.agent(&session, "inspect_page", json!({"selector": "#main-frame-error"})).await.ok();
        read.filter(|read| {
            read["isError"] != true && read["content"][0]["text"].as_str().is_some_and(|t| t.contains("\"count\":1"))
        })
    });
    assert!(failed.is_some(), "the browser never showed its error page");
    let listed = admin.agent(&session, "list_windows", json!({})).await.expect("the windows");
    let windows = listed["structuredContent"]["windows"].as_array().expect("its windows");
    let (pid, window) = common::first_window(windows).unwrap_or_else(|| panic!("a window with no size: {listed}"));
    let read =
        admin.agent(&session, "get_window_state", json!({"pid": pid, "window_id": window})).await.expect("a read");
    assert_ne!(read["isError"], true, "the window as listed is not one the driver reads: {read}");
    assert!(read["content"][0]["text"].as_str().is_some_and(|text| text.contains("no-such-host.invalid")), "{read}");
    admin.stop(&session).await;
}

/// A session may start on an empty page (`about:blank`), which asks the network for nothing. With Developer tools, the
/// driver binds its window exactly, and a first navigation that turns into a download (the tab stays where it was,
/// and Chrome opens its downloads bubble) leaves it bound: on an error page the bubble's window costs the binding.
#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_session_started_on_an_empty_page_keeps_its_window_bound_through_a_download() {
    let admin = Admin::login().await;
    let session = admin.create_with(json!({"url": "about:blank", "devtools": true})).await;
    let (pid, window) = admin.wait_windows(&session, "the empty page's window", common::first_window).await;
    let native = json!({"pid": pid, "window_id": window, "session": "blank"});
    let mut prepare = native.clone();
    prepare["strategy"] = json!({"kind": "existing_profile"});
    let prepared = admin.agent(&session, "browser_prepare", prepare).await.expect("the grant");
    assert_eq!(prepared["structuredContent"]["status"], "ok", "{prepared}");
    let bound = admin.agent(&session, "get_browser_state", native.clone()).await.expect("the window's tabs");
    let bound = &bound["structuredContent"];
    assert_eq!(bound["binding_quality"], "exact", "{bound}");
    let tab = bound["tabs"].as_array().and_then(|tabs| tabs.iter().find(|tab| tab["active"] == true)).expect("a tab");
    assert_eq!(tab["url"], "about:blank", "{bound}");
    let download = json!({
        "session": "blank", "target_id": bound["target_id"], "tab_id": tab["tab_id"],
        "url": "https://fixture.bench/big?size=4096",
    });
    // The navigation ends as a download, which the driver says as a navigation that did not happen: the file the
    // session then holds says it was one.
    let _ = admin.agent(&session, "browser_navigate", download).await;
    let downloaded = common::until!(60, Duration::from_millis(500), {
        let files = admin.agent(&session, "list_files", json!({})).await.expect("the session's files");
        (files["structuredContent"]["downloads"][0]["name"] == "big.bin").then_some(())
    });
    assert!(downloaded.is_some(), "the navigation downloaded nothing");
    // While Chrome shows its downloads bubble, and after.
    for _ in 0..8 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let after = admin.agent(&session, "get_browser_state", native.clone()).await.expect("the window's tabs");
        let after = &after["structuredContent"];
        assert_eq!((&after["binding_quality"], &after["mutation_allowed"]), (&json!("exact"), &json!(true)), "{after}");
    }
    admin.stop(&session).await;
}

/// While a tab plays sound, Chrome names its window "Audio playing" to assistive technology, by which the driver finds
/// a window's process: the agent's list keeps the window's process and number all the same.
#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_window_keeps_its_process_in_the_agents_list_while_its_tab_plays_sound() {
    let admin = Admin::login().await;
    let session = admin.create("https://fixture.bench/noisy-tone.html").await;
    admin.wait_title(&session, "Noisy tone").await;
    let (pid, window) = admin.wait_windows(&session, "the page's window", common::first_window).await;
    let target = json!({"pid": pid, "window_id": window});
    admin.agent(&session, "get_window_state", target.clone()).await.expect("the window read");
    // The page's tone plays for 6 s from this click.
    let click = json!({"pid": pid, "window_id": window, "x": 300, "y": 300});
    let clicked = admin.agent(&session, "click", click).await.expect("a click on the page");
    assert_ne!(clicked["isError"], true, "{clicked}");
    for _ in 0..8 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let read = admin.agent(&session, "list_windows", json!({})).await.expect("the windows");
        let listed = common::first_window(read["structuredContent"]["windows"].as_array().expect("its windows"));
        assert_eq!(listed, Some((pid, window)), "{read}");
    }
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
    // Nor in what a navigation reads there.
    let same = json!({"url": format!("{FIXTURE}#typed"), "read": {"selector": "#echo"}});
    let went = admin.agent(&session, "navigate", same).await.expect("the same page");
    let text = went.to_string();
    assert!(
        !text.contains("hunter2secret") && went["structuredContent"]["navigation"]["outcome"] == "same_document",
        "{text}"
    );
    // Typed with a space after it, which the paragraph that shows it drops: it is found there all the same.
    let keys = async || {
        let counts = admin.agent(&session, "inspect_page", json!({"selector": "#counts"})).await.expect("the counts");
        let text = counts["structuredContent"]["elements"][0]["text"].as_str().unwrap_or_default().to_owned();
        common::count(&text, "keys").unwrap_or_else(|| panic!("no count of keys: {counts}"))
    };
    let before = keys().await;
    let held = admin.take(&session, &person).await;
    person.click(held, common::FIELD.0, common::FIELD.1).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    person.chord(held, &[display_protocol::keys::SPACE]).await;
    person.type_keys(held, "swordfish7").await;
    person.chord(held, &[display_protocol::keys::SPACE]).await;
    tokio::time::sleep(Duration::from_millis(800)).await;
    admin.hand_back(&session, &person).await;
    // The page had every key of it (a space, ten characters, a space) before control went back.
    assert_eq!(keys().await, before + 12, "the page did not get what was typed");
    let read = admin.agent(&session, "inspect_page", json!({"selector": "#echo"})).await.expect("the echo");
    let text = read.to_string();
    assert!(!text.contains("swordfish7") && !text.contains("hunter2secret"), "{text}");
    // Nor where the page shows it with every space it was typed with.
    let read = admin.agent(&session, "inspect_page", json!({"selector": "#exact"})).await.expect("as typed");
    let text = read.to_string();
    assert!(!text.contains("swordfish7") && !text.contains("hunter2secret") && text.contains("[redacted]"), "{text}");
    person.close().await;
    admin.stop(&session).await;
}

/// A session a person started with Developer tools runs an agent's scripts in the page's own world.
#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn developer_tools_let_an_agent_run_a_script_on_the_page() {
    let admin = Admin::login().await;
    let session = admin.create_with(json!({"url": "https://fixture.bench/inspect.html", "devtools": true})).await;
    // The page itself, loaded and shown: a script asked before that has no page to run on, and says so (as it did
    // once in two rounds with each browser held to half a processor).
    admin.wait_title(&session, "Inspect").await;
    let (pid, window) = admin.wait_windows(&session, "the page's window", common::first_window).await;
    let script = json!({"javascript": "document.title + 40 + 2", "pid": pid, "window_id": window});
    let ran = admin.agent(&session, "run_javascript", script).await.expect("the script");
    assert!(ran.to_string().contains("Inspect402"), "{ran}");
    // What a script answers reads as it always did: the value as JSON after the label, a promise's once it settles,
    // an exception by its description, and a thing that is no value by what it is called.
    let run = |javascript: String| {
        let (admin, session) = (&admin, &session);
        async move {
            let script = json!({"javascript": javascript, "pid": pid, "window_id": window});
            let ran = admin.agent(session, "run_javascript", script).await.expect("the script");
            ran["content"][0]["text"].as_str().unwrap_or_else(|| panic!("no text in {ran}")).to_owned()
        }
    };
    let label = "cdp.runtime.evaluate.user_gesture: ";
    assert_eq!(run("document.title + 40 + 2".into()).await, format!("{label}\"Inspect402\""));
    assert_eq!(run("new Promise((done) => setTimeout(() => done(7), 300))".into()).await, format!("{label}7"));
    let thrown = run("nowhere()".into()).await;
    assert!(thrown.starts_with(&format!("{label}exception: ReferenceError")), "{thrown}");
    assert_eq!(run("({a: 1})".into()).await, format!("{label}Object"));
    // A script longer than one frame of the link carries goes in parts, whole.
    let long = format!("/* {} */ 'é'.length + 1", "padding ".repeat(20_000));
    assert_eq!(run(long).await, format!("{label}2"));
    // It still runs once the driver's page tools are bound to the browser.
    let native = json!({"pid": pid, "window_id": window, "session": "script"});
    let mut prepare = native.clone();
    prepare["strategy"] = json!({"kind": "existing_profile"});
    admin.agent(&session, "browser_prepare", prepare).await.expect("the grant");
    admin.agent(&session, "get_browser_state", native).await.expect("the window's tabs");
    assert_eq!(run("1 + 1".into()).await, format!("{label}2"));
    // Reading a page is the same here.
    let read = admin.agent(&session, "inspect_page", json!({"selector": "h1"})).await.expect("the heading");
    assert_eq!(read["structuredContent"]["elements"][0]["attributes"]["id"], "top", "{read}");
    admin.stop(&session).await;
}

/// A person who takes control while an agent's script still runs gets control at once: the script is given up, its
/// call answers that a person holds control, no script runs while they do, and scripts run again once they give
/// control back.
#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_takeover_gives_up_a_script_that_runs_and_none_runs_while_a_person_holds_control() {
    let admin = Admin::login().await;
    let session = admin.create_with(json!({"url": "https://fixture.bench/inspect.html", "devtools": true})).await;
    admin.wait_title(&session, "Inspect").await;
    let (pid, window) = admin.wait_windows(&session, "the page's window", common::first_window).await;
    let person = admin.watch(&session, 0).await;
    person.wait_lease(LeaseHolder::Agent).await;
    let script = |javascript: &str| json!({"javascript": javascript, "pid": pid, "window_id": window});
    // A script that waits twenty seconds for its promise.
    let slow = script("new Promise((done) => setTimeout(() => done('late'), 20000))");
    let running = admin.agent(&session, "run_javascript", slow);
    let taking = async {
        tokio::time::sleep(Duration::from_secs(2)).await;
        let began = std::time::Instant::now();
        let held = admin.take(&session, &person).await;
        (held, began.elapsed())
    };
    let (ran, (_held, took)) = tokio::join!(running, taking);
    assert!(took < Duration::from_secs(4), "control came after {took:?}");
    assert_eq!(ran.expect_err("given up for the person").0, 409);
    assert_eq!(admin.agent(&session, "run_javascript", script("1 + 1")).await.expect_err("refused").0, 409);
    admin.hand_back(&session, &person).await;
    let after = admin.agent(&session, "run_javascript", script("1 + 1")).await.expect("a script again");
    assert_eq!(after["content"][0]["text"], "cdp.runtime.evaluate.user_gesture: 2", "{after}");
    person.close().await;
    admin.stop(&session).await;
}

/// With several tabs, a script runs on the page of the one that shows: the page a person watching would point at.
#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_script_runs_on_the_page_of_the_tab_that_shows() {
    let admin = Admin::login().await;
    let session = admin.create_with(json!({"url": "https://fixture.bench/inspect.html", "devtools": true})).await;
    admin.wait_title(&session, "Inspect").await;
    let (pid, window) = admin.wait_windows(&session, "the page's window", common::first_window).await;
    let title = || {
        let (admin, session) = (&admin, &session);
        async move {
            let script = json!({"javascript": "document.title", "pid": pid, "window_id": window});
            let ran = admin.agent(session, "run_javascript", script).await.expect("the script");
            ran["content"][0]["text"].as_str().unwrap_or_else(|| panic!("no text in {ran}")).to_owned()
        }
    };
    let label = "cdp.runtime.evaluate.user_gesture: ";
    assert_eq!(title().await, format!("{label}\"Inspect\""));
    // A second tab, on another page, now shows.
    let keys = |keys: &[&str]| json!({"pid": pid, "window_id": window, "keys": keys});
    admin.agent(&session, "hotkey", keys(&["ctrl", "t"])).await.expect("a new tab");
    admin.agent(&session, "hotkey", keys(&["ctrl", "l"])).await.expect("its address bar");
    let address = json!({"pid": pid, "window_id": window, "text": format!("{}\n", common::ARTICLE)});
    admin.agent(&session, "type_text", address).await.expect("an address typed");
    let mut seen = String::new();
    let second = common::until!(60, Duration::from_millis(500), {
        seen = title().await;
        (seen == format!("{label}\"Static article\"")).then_some(())
    });
    assert!(second.is_some(), "the script ran on {seen}, not on the tab that shows");
    // Back on the first tab, its page.
    admin.agent(&session, "hotkey", keys(&["ctrl", "1"])).await.expect("the first tab");
    let first = common::until!(60, Duration::from_millis(500), {
        seen = title().await;
        (seen == format!("{label}\"Inspect\"")).then_some(())
    });
    assert!(first.is_some(), "the script ran on {seen}, not on the tab that shows");
    admin.stop(&session).await;
}

/// A script that waits for ever (a promise nothing settles) is given up when its time is over (the display's wait for
/// it, on a page that says nothing more): its call says so, the next script runs, and a person takes control as ever.
/// (One that never yields is another matter: see the test below.)
#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_script_that_never_ends_is_given_up_and_the_session_goes_on() {
    let admin = Admin::login().await;
    let session = admin.create_with(json!({"url": "https://fixture.bench/inspect.html", "devtools": true})).await;
    admin.wait_title(&session, "Inspect").await;
    let (pid, window) = admin.wait_windows(&session, "the page's window", common::first_window).await;
    let script = |javascript: &str| json!({"javascript": javascript, "pid": pid, "window_id": window});
    let began = std::time::Instant::now();
    let ran = admin.agent(&session, "run_javascript", script("new Promise(() => {})")).await;
    let took = began.elapsed();
    let ran = ran.expect("an answer, of a script given up");
    assert_eq!(ran["isError"], true, "{ran}");
    assert!(ran["content"][0]["text"].as_str().is_some_and(|said| said.contains("took too long")), "{ran}");
    assert!(took < Duration::from_secs(50), "given up only after {took:?}");
    let after = admin.agent(&session, "run_javascript", script("1 + 1")).await.expect("a script again");
    assert_eq!(after["content"][0]["text"], "cdp.runtime.evaluate.user_gesture: 2", "{after}");
    let person = admin.watch(&session, 0).await;
    admin.take(&session, &person).await;
    person.close().await;
    admin.stop(&session).await;
}

/// A script that runs on without yielding (a loop that does not end) keeps its page from answering anything, to a
/// person as to a later script. The browser ends it when its thirty seconds are over: its page then runs scripts
/// again, and what the page had going goes on. A script that only waits ends nothing, then or later: one given up
/// leaves no time limit behind it that a later script would meet.
#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_script_that_never_yields_is_ended_in_its_page_when_its_time_is_over() {
    let admin = Admin::login().await;
    let session = admin.create_with(json!({"url": "https://fixture.bench/inspect.html", "devtools": true})).await;
    admin.wait_title(&session, "Inspect").await;
    let (pid, window) = admin.wait_windows(&session, "the page's window", common::first_window).await;
    let person = admin.watch(&session, 0).await;
    person.wait_lease(LeaseHolder::Agent).await;
    let script = |javascript: &str| json!({"javascript": javascript, "pid": pid, "window_id": window});
    let label = "cdp.runtime.evaluate.user_gesture: ";
    let read = |javascript: &'static str| {
        let (admin, session) = (&admin, &session);
        async move {
            let ran = admin.agent(session, "run_javascript", script(javascript)).await.expect("the script");
            ran["content"][0]["text"].as_str().unwrap_or_else(|| panic!("no text in {ran}")).to_owned()
        }
    };
    let count = |text: String| text.trim_start_matches(label).parse::<u64>().unwrap_or_else(|_| panic!("{text}"));
    // A counter the page keeps going by itself, then a script that waits for ever, given up for a person two seconds
    // in: the counter goes on after.
    assert_eq!(read("window.kept = 0; setInterval(() => window.kept++, 50); 'set'").await, format!("{label}\"set\""));
    let waited = std::time::Instant::now();
    let running = admin.agent(&session, "run_javascript", script("new Promise(() => {})"));
    let taking = async {
        tokio::time::sleep(Duration::from_secs(2)).await;
        admin.take(&session, &person).await
    };
    let (ran, _) = tokio::join!(running, taking);
    assert_eq!(ran.expect_err("given up for the person").0, 409);
    admin.hand_back(&session, &person).await;
    let before = count(read("window.kept").await);
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(count(read("window.kept").await) > before, "what the page set going stopped with a script that waited");
    // A script that runs for ten seconds without yielding, across the moment the first one's thirty seconds end:
    // it runs whole, so no time limit was left behind.
    tokio::time::sleep(Duration::from_secs(24).saturating_sub(waited.elapsed())).await;
    let whole = read("const end = Date.now() + 10000; while (Date.now() < end) {} 'whole'").await;
    assert_eq!(whole, format!("{label}\"whole\""), "after {:?}", waited.elapsed());
    assert!(waited.elapsed() > Duration::from_secs(33), "it ended before the first one's time was over");
    // A loop that does not end: its call answers only once its time is over, as an error; its page then answers
    // again, its own timer still going.
    let began = std::time::Instant::now();
    let ended = admin.agent(&session, "run_javascript", script("while (true) {}")).await.expect("an answer");
    let took = began.elapsed();
    assert_eq!(ended["isError"], true, "{ended}");
    assert!(
        took > Duration::from_secs(25) && took < Duration::from_secs(50),
        "the loop's call answered after {took:?}"
    );
    let before = count(read("window.kept").await);
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(count(read("window.kept").await) > before, "the page's own timer stopped with the script");
    person.close().await;
    admin.stop(&session).await;
}

/// A session with Developer tools on a page that `script` filled, bound for page reads: what a read of its tab takes
/// (`get_browser_state`'s arguments, the semantic format among them). The page is filled first: once the driver is
/// bound to the browser, the script tool no longer runs.
async fn page_to_read(admin: &Admin, script: &str) -> (String, Value) {
    let session = admin.create_with(json!({"url": "https://fixture.bench/inspect.html", "devtools": true})).await;
    // The page itself, loaded: what a script wrote before that would go with the page it wrote in.
    admin.wait_title(&session, "Inspect").await;
    let (pid, window) = admin.wait_windows(&session, "the page's window", common::first_window).await;
    let script = json!({"javascript": script, "pid": pid, "window_id": window});
    admin.agent(&session, "run_javascript", script).await.expect("the page's content");
    let native = json!({"pid": pid, "window_id": window, "session": "read"});
    let mut prepare = native.clone();
    prepare["strategy"] = json!({"kind": "existing_profile"});
    let prepared = admin.agent(&session, "browser_prepare", prepare).await.expect("the grant");
    assert_eq!(prepared["structuredContent"]["status"], "ok", "{prepared}");
    let bound = admin.agent(&session, "get_browser_state", native).await.expect("the window's tabs");
    let bound = &bound["structuredContent"];
    let tab = bound["tabs"].as_array().and_then(|tabs| tabs.iter().find(|tab| tab["active"] == true)).expect("a tab");
    let page = json!({
        "session": "read", "target_id": bound["target_id"], "tab_id": tab["tab_id"], "snapshot_format": "semantic_v2",
    });
    (session, page)
}

/// A read of `page`, repeated until the page holds more than `least` elements and as many as the read before it:
/// what a script wrote reaches Chrome's accessibility tree a moment later, and not all at once.
async fn read_page(admin: &Admin, session: &str, page: &Value, least: u64) -> Value {
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    let mut before = None;
    loop {
        let read = admin.agent(session, "get_browser_state", page.clone()).await.expect("a read");
        let read = read["structuredContent"].clone();
        let total = read["snapshot"]["total_nodes"].as_u64();
        if total.is_some_and(|total| total > least) && total == before {
            return read;
        }
        before = total;
        assert!(std::time::Instant::now() < deadline, "the page never held {least} elements: {}", read["snapshot"]);
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

/// A page read in a session with Developer tools selects 300 elements, and says how many it left out; the rest comes
/// by its continuation, each asked at once after the read before: the pieces hold as many elements as the page has,
/// and each entry's title, price and button is in one of them.
#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_page_larger_than_one_read_comes_whole_by_its_continuations() {
    let admin = Admin::login().await;
    // A catalog of 200 entries, each a link, a price and a button.
    let catalog = "const list = document.createElement('ul'); for (let n = 0; n < 200; n++) { \
        const item = document.createElement('li'); \
        item.innerHTML = `<a href=\"#b${n}\">Book ${n}</a> <span>£${n}.50</span> <button>Add ${n}</button>`; \
        list.append(item); } document.body.append(list); 'listed'";
    let (session, page) = page_to_read(&admin, catalog).await;
    let first = read_page(&admin, &session, &page, 600).await;
    let total = first["snapshot"]["total_nodes"].as_u64().expect("a total");
    assert_eq!(first["snapshot"]["selected_nodes"], json!(300), "{}", first["snapshot"]);
    assert_eq!(first["snapshot"]["omitted"]["budget"], json!(total - 300));
    let (mut selected, mut outlines) = (300, vec![first["outline"].as_str().expect("an outline").to_owned()]);
    let mut continuation = first["snapshot"]["continuation"].clone();
    while let Some(token) = continuation.as_str() {
        let mut next = page.clone();
        next["continuation"] = json!(token);
        let piece = admin.agent(&session, "get_browser_state", next).await.expect("the next piece");
        let piece = &piece["structuredContent"];
        assert_eq!(piece["status"], "ok", "{piece}");
        selected += piece["snapshot"]["selected_nodes"].as_u64().expect("its count");
        outlines.push(piece["outline"].as_str().expect("an outline").to_owned());
        continuation = piece["snapshot"]["continuation"].clone();
        assert!(outlines.len() <= 20, "the continuations never ended");
    }
    assert_eq!(selected, total, "the pieces hold as many elements as the page has");
    let whole = outlines.join("\n");
    for n in 0..200 {
        for part in [format!("\"Book {n}\""), format!("\"£{n}.50\""), format!("\"Add {n}\"")] {
            assert!(whole.contains(&part), "{part} is in no piece");
        }
    }
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
