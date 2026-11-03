//! A session's chat, through the running application: the container it runs in, what its MCP socket reaches, and its
//! terminal. The program itself comes from its vendor's installer, whatever the network lets through: nothing here
//! needs it to have arrived.

mod common;

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::time::Duration;

use common::{Admin, INSTANCE, TYPING, docker};
use futures_util::{SinkExt, StreamExt};
use reqwest::Method;
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;

const PREFIX: &str = "e2e chat ";

async fn account(admin: &Admin, name: &str) -> String {
    let name = format!("{PREFIX}{name}");
    let listed = admin.get("/api/chat-accounts").await.expect("the accounts");
    for old in listed["accounts"].as_array().unwrap().iter().filter(|a| a["name"] == name.as_str()) {
        let path = format!("/api/chat-accounts/{}", old["id"].as_str().unwrap());
        admin.call(Method::DELETE, &path, None).await.expect("an earlier run's account goes");
    }
    let body = json!({"name": name, "product": "codex"});
    let created = admin.call(Method::POST, "/api/chat-accounts", Some(body)).await.expect("an account");
    created["id"].as_str().unwrap().to_owned()
}

fn runner(session: &str) -> String {
    format!("{INSTANCE}-{session}-runner")
}

fn inspected(container: &str) -> Value {
    let (ok, text) = docker(&["inspect", container]);
    assert!(ok, "{text}");
    serde_json::from_str::<Value>(&text).unwrap()[0].clone()
}

/// What the chat's own MCP socket answers `requests` with, as `user` in its container, by id.
fn mcp(container: &str, user: &str, requests: &[Value]) -> Vec<Value> {
    let mut child = Command::new("docker")
        .args(["exec", "-i", "-u", user, container, "/usr/local/bin/runner-mcp"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("docker runs");
    let mut stdin = child.stdin.take().unwrap();
    for request in requests {
        writeln!(stdin, "{request}").unwrap();
    }
    stdin.flush().unwrap();
    // Open until every answer came: the relay closes soon after its input ends. One refused gets none, and ends here.
    let stdout = BufReader::new(child.stdout.take().unwrap());
    let answers: Vec<Value> = stdout
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| serde_json::from_str(&line).ok())
        .take(requests.len())
        .collect();
    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
    answers
}

fn answer(answers: &[Value], id: i64) -> Value {
    answers.iter().find(|a| a["id"] == id).unwrap_or_else(|| panic!("no answer {id} in {answers:?}"))["result"].clone()
}

fn call(id: i64, name: &str, arguments: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": name, "arguments": arguments}})
}

/// Alone: each holds a session beside another's, and the sessions at once are counted (scripts/e2e.sh).
mod exclusive {
    use super::*;

    #[tokio::test]
    #[ignore = "needs the bench stack: scripts/e2e.sh"]
    async fn a_chat_runs_locked_down_and_reaches_its_own_session_alone() {
        let admin = Admin::login().await;
        let session = admin.create(TYPING).await;
        let other =
            admin.call(Method::POST, "/api/sessions", Some(json!({"url": TYPING, "devtools": false}))).await.unwrap();
        let other = other["id"].as_str().unwrap().to_owned();
        admin.adopt(&other);
        let id = account(&admin, "locked").await;
        let chat = format!("/api/sessions/{session}/chat");
        let started = admin.call(Method::POST, &chat, Some(json!({"account": id}))).await.expect("a chat");
        assert_eq!(started["chat"]["product"], "codex");
        let listed = admin.get("/api/sessions").await.unwrap();
        let mine = listed["sessions"].as_array().unwrap().iter().find(|s| s["id"] == session.as_str()).unwrap().clone();
        assert_eq!(mine["chat"]["account"], id.as_str());

        // One chat for a session, one for an account.
        let again = admin.call(Method::POST, &chat, Some(json!({"account": id}))).await.expect_err("a second chat");
        assert_eq!(again.0, 409);
        let elsewhere = format!("/api/sessions/{other}/chat");
        let taken =
            admin.call(Method::POST, &elsewhere, Some(json!({"account": id}))).await.expect_err("the same account");
        assert_eq!(taken.0, 409);
        let in_use = admin.call(Method::DELETE, &format!("/api/chat-accounts/{id}"), None).await;
        assert_eq!(in_use.expect_err("an account in use stays").0, 409);

        // The container: no network, no privilege, no Docker, the home and the two sockets' directories and nothing else.
        let container = runner(&session);
        let found = inspected(&container);
        let host = &found["HostConfig"];
        assert_eq!(host["NetworkMode"], "none");
        assert_eq!(host["ReadonlyRootfs"], true);
        assert_eq!(host["Privileged"], false);
        assert_eq!(host["CapDrop"], json!(["ALL"]));
        assert_eq!(found["Config"]["User"], "10007:10007");
        // What it serves itself never goes through its proxy, which refuses this address.
        let env = found["Config"]["Env"].to_string();
        assert!(env.contains("NO_PROXY=localhost,127.0.0.1,::1"), "{env}");
        let mut mounts: Vec<(String, bool)> = found["Mounts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| (m["Destination"].as_str().unwrap().to_owned(), m["RW"].as_bool().unwrap()))
            .collect();
        mounts.sort();
        let expected = [("/home/runner", true), ("/run/chat/mcp", false), ("/run/chat/proxy", false)];
        assert_eq!(mounts, expected.map(|(path, rw)| (path.to_owned(), rw)));

        // What its MCP socket reaches: this session, with no token and no allow-list.
        let initialize = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "e2e", "version": "1"}}});
        let answers = mcp(
            &container,
            "10007:10007",
            &[
                initialize.clone(),
                json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
                call(3, "list_sessions", json!({})),
                call(4, "session_status", json!({"session": session})),
                call(5, "session_status", json!({"session": other})),
                call(6, "start_session", json!({"url": TYPING})),
                call(7, "stop_session", json!({"session": session})),
                call(8, "get_desktop_state", json!({"session": other})),
                call(9, "session_status", json!({})),
                call(10, "get_desktop_state", json!({})),
            ],
        );
        assert!(answer(&answers, 1)["instructions"].as_str().unwrap().contains(&session));
        let text = |id| answer(&answers, id).to_string();
        let tools = answer(&answers, 2);
        let names: Vec<&str> = tools["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"session_status") && names.contains(&"get_desktop_state"), "{names:?}");
        // A chat has one session: its tools do not ask which, and work with no `session` at all.
        for tool in tools["tools"].as_array().unwrap() {
            assert!(tool["inputSchema"]["properties"].get("session").is_none(), "{tool}");
            assert!(!tool["inputSchema"]["required"].to_string().contains("session"), "{tool}");
        }
        assert_ne!(answer(&answers, 9)["isError"], true, "{}", text(9));
        assert!(text(9).contains(&session) || answer(&answers, 9)["structuredContent"].is_object(), "{}", text(9));
        assert_ne!(answer(&answers, 10)["isError"], true, "{}", text(10));
        for absent in ["start_session", "stop_session", "list_saved_sessions"] {
            assert!(!names.contains(&absent), "{absent} in {names:?}");
        }
        assert!(text(3).contains(&session) && !text(3).contains(&other), "{}", text(3));
        assert_ne!(answer(&answers, 4)["isError"], true, "{}", text(4));
        for refused in [5, 6, 7, 8] {
            assert_eq!(answer(&answers, refused)["isError"], true, "{}", text(refused));
        }
        assert!(
            admin.get("/api/sessions").await.unwrap()["sessions"].as_array().unwrap().len() >= 2,
            "nothing was stopped"
        );

        // Only the runner's uid is served.
        assert!(mcp(&container, "10008:10014", &[initialize]).is_empty(), "another uid is refused");

        // Its end: the containers go, the home stays until the account goes.
        let home = format!("{INSTANCE}-account-{id}");
        assert!(docker(&["volume", "inspect", &home]).0, "the account's home is there");
        admin.call(Method::DELETE, &chat, None).await.expect("the chat ends");
        assert!(!docker(&["inspect", &container]).0, "the runner went");
        assert!(!docker(&["inspect", &format!("{container}-proxy")]).0, "its proxy went");
        assert!(docker(&["volume", "inspect", &home]).0, "the home stays");
        admin.call(Method::POST, &chat, Some(json!({"account": id}))).await.expect("the account opens again");
        admin.stop(&session).await;
        assert!(!docker(&["inspect", &container]).0, "stopping the session ends its chat");
        assert!(docker(&["volume", "inspect", &home]).0, "and the home stays");
        admin.call(Method::DELETE, &format!("/api/chat-accounts/{id}"), None).await.expect("the account goes");
        assert!(!docker(&["volume", "inspect", &home]).0, "with its home");
        admin.clear().await;
    }

    #[tokio::test]
    #[ignore = "needs the bench stack: scripts/e2e.sh"]
    async fn a_chats_terminal_is_sized_replaced_and_ends_with_its_program() {
        let admin = Admin::login().await;
        let session = admin.create(TYPING).await;
        let id = account(&admin, "terminal").await;
        let chat = format!("/api/sessions/{session}/chat");
        admin.call(Method::POST, &chat, Some(json!({"account": id}))).await.expect("a chat");
        let container = runner(&session);

        let mut first = admin.socket(&format!("/ws/sessions/{session}/chat")).await;
        let painted = tokio::time::timeout(Duration::from_secs(30), first.next()).await.expect("the terminal paints");
        assert!(matches!(painted, Some(Ok(Message::Binary(_)))), "{painted:?}");

        first.send(Message::Text(json!({"cols": 97, "rows": 31}).to_string().into())).await.unwrap();
        let width =
            || docker(&["exec", &container, "tmux", "display-message", "-p", "#{client_width}x#{client_height}"]).1;
        let mut sized = String::new();
        for _ in 0..50 {
            sized = width();
            if sized.trim() == "97x31" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert_eq!(sized.trim(), "97x31");

        // A later terminal takes its place, and the older one says so.
        let mut second = admin.socket(&format!("/ws/sessions/{session}/chat")).await;
        let closed = async {
            loop {
                match first.next().await {
                    Some(Ok(Message::Close(Some(frame)))) => return u16::from(frame.code),
                    Some(Ok(_)) => {}
                    other => panic!("{other:?}"),
                }
            }
        };
        assert_eq!(tokio::time::timeout(Duration::from_secs(30), closed).await.expect("it closes"), 4001);

        // The program ending ends the chat, the home staying.
        assert!(docker(&["exec", &container, "tmux", "kill-server"]).0);
        let closed = async {
            loop {
                match second.next().await {
                    Some(Ok(Message::Close(Some(frame)))) => return u16::from(frame.code),
                    Some(Ok(_)) => {}
                    other => panic!("{other:?}"),
                }
            }
        };
        assert_eq!(tokio::time::timeout(Duration::from_secs(30), closed).await.expect("it closes"), 4000);
        let listed = admin.get("/api/sessions").await.unwrap();
        let mine = listed["sessions"].as_array().unwrap().iter().find(|s| s["id"] == session.as_str()).unwrap().clone();
        assert!(mine["chat"].is_null(), "{mine}");
        assert!(docker(&["volume", "inspect", &format!("{INSTANCE}-account-{id}")]).0);
        admin.call(Method::DELETE, &format!("/api/chat-accounts/{id}"), None).await.expect("the account goes");
        admin.clear().await;
    }
}
