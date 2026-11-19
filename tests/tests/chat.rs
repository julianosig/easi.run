//! A session's chat, through the running application: the container it runs in, what its MCP socket reaches, and its
//! terminal. The program itself comes from its vendor's installer, whatever the network lets through: nothing here
//! needs it to have arrived.

mod common;

use std::time::Duration;

use common::{Admin, INSTANCE, TYPING, answer, call, docker, mcp};
use futures_util::{SinkExt, StreamExt};
use reqwest::Method;
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;

const PREFIX: &str = "e2e chat ";

async fn account(admin: &Admin, name: &str) -> String {
    account_of(admin, name, "codex").await
}

async fn account_of(admin: &Admin, name: &str, product: &str) -> String {
    let name = format!("{PREFIX}{name}");
    let listed = admin.get("/api/chat-accounts").await.expect("the accounts");
    for old in listed["accounts"].as_array().unwrap().iter().filter(|a| a["name"] == name.as_str()) {
        let path = format!("/api/chat-accounts/{}", old["id"].as_str().unwrap());
        admin.call(Method::DELETE, &path, None).await.expect("an earlier run's account goes");
    }
    let body = json!({"name": name, "product": product});
    let created = admin.call(Method::POST, "/api/chat-accounts", Some(body)).await.expect("an account");
    created["id"].as_str().unwrap().to_owned()
}

fn runner(session: &str) -> String {
    format!("{INSTANCE}-{session}-runner")
}

/// A place's folder, which nothing but the uninstall removes: a test's own goes with the test.
fn folder(place: &str) -> String {
    format!("{INSTANCE}-folder-{place}")
}

fn forget_folder(place: &str) {
    let (gone, said) = docker(&["volume", "rm", &folder(place)]);
    assert!(gone, "{said}");
}

/// What `script` prints in the chat's container, as its own user.
fn in_chat(session: &str, script: &str) -> String {
    let (ok, said) = docker(&["exec", "--user", "10007:10007", &runner(session), "bash", "-c", script]);
    assert!(ok, "{script}: {said}");
    said
}

fn inspected(container: &str) -> Value {
    let (ok, text) = docker(&["inspect", container]);
    assert!(ok, "{text}");
    serde_json::from_str::<Value>(&text).unwrap()[0].clone()
}

/// What the chat's own MCP socket answers `requests` with, as `user` in its container, by id.
/// Alone: each holds a session beside another's, and the sessions at once are counted (scripts/e2e.sh).
mod exclusive {
    use super::*;

    /// A session that had a chat open opens it again as it starts again after the admin was stopped (an update): with
    /// the same account, and in the place of its home it worked in, where its program finds its conversation.
    #[tokio::test]
    #[ignore = "needs the bench stack: scripts/e2e.sh"]
    async fn a_chat_opens_again_with_its_session_in_the_place_it_had() {
        let admin = Admin::login().await;
        let session = admin.create(TYPING).await;
        let id = account(&admin, "again").await;
        let chat = format!("/api/sessions/{session}/chat");
        admin.call(Method::POST, &chat, Some(json!({"account": id}))).await.expect("a chat");
        let place = format!("RUNNER_PLACE={session}");
        assert!(inspected(&runner(&session))["Config"]["Env"].to_string().contains(&place));

        let (restarted, said) = docker(&["restart", "--time", "120", common::ADMIN_CONTAINER]);
        assert!(restarted, "{said}");
        let back = common::until!(120, Duration::from_secs(1), common::answers().await.then_some(()));
        assert!(back.is_some(), "the admin is back");
        let admin = Admin::login().await;
        let again = admin.resumed().await;
        let again = again.iter().find(|s| s["was"] == json!([session])).expect("the session started again");
        assert_eq!(again["chat"]["account"], id.as_str(), "its chat, with its account: {again}");
        // Under its new id, the chat works where it did: the session's place is the one it first had.
        let now = again["id"].as_str().expect("an id");
        let env = inspected(&runner(now))["Config"]["Env"].to_string();
        assert!(env.contains(&place) && env.contains("RUNNER_PRODUCT=codex"), "{env}");
        admin.clear().await;
        admin.call(Method::DELETE, &format!("/api/chat-accounts/{id}"), None).await.expect("the account goes");
        forget_folder(&session);
    }

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

        // One chat for a session, and an account stays while one is open.
        let again = admin.call(Method::POST, &chat, Some(json!({"account": id}))).await.expect_err("a second chat");
        assert_eq!(again.0, 409);
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
        // An unsaved session's chat works in that session's own place in the home.
        assert!(env.contains(&format!("RUNNER_PLACE={session}")), "{env}");
        assert_eq!(host["Memory"], 2048_i64 << 20);
        let mut mounts: Vec<(String, bool)> = found["Mounts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| (m["Destination"].as_str().unwrap().to_owned(), m["RW"].as_bool().unwrap()))
            .collect();
        mounts.sort();
        // The home, the session's folder over its place in it (and where each takes its ownership or shows what it
        // had before), and the two sockets' directories.
        let place = format!("/home/runner/work/{session}");
        let expected = [
            ("/home/runner", true),
            (place.as_str(), true),
            ("/run/chat/home", true),
            ("/run/chat/mcp", false),
            ("/run/chat/proxy", false),
            ("/run/chat/work", true),
        ];
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

        // The same account in the other session: one home, and each chat reaches its own session alone.
        let elsewhere = format!("/api/sessions/{other}/chat");
        admin.call(Method::POST, &elsewhere, Some(json!({"account": id}))).await.expect("the same account");
        let accounts = admin.get("/api/chat-accounts").await.unwrap();
        let shown = accounts["accounts"].as_array().unwrap().iter().find(|a| a["id"] == id.as_str()).unwrap().clone();
        assert_eq!(shown["sessions"].as_array().unwrap().len(), 2, "{shown}");
        let second = runner(&other);
        let home = |container: &str| -> Value {
            let mounts = inspected(container)["Mounts"].clone();
            mounts.as_array().unwrap().iter().find(|m| m["Destination"] == "/home/runner").unwrap()["Name"].clone()
        };
        assert_eq!(home(&container), home(&second));
        let answers = mcp(
            &second,
            "10007:10007",
            &[
                initialize.clone(),
                call(2, "list_sessions", json!({})),
                call(3, "session_status", json!({"session": session})),
            ],
        );
        let listed = answer(&answers, 2).to_string();
        assert!(listed.contains(&other) && !listed.contains(&session), "{listed}");
        assert_eq!(answer(&answers, 3)["isError"], true, "{answers:?}");
        // Ending one leaves the other, and the account stays until the last.
        admin.call(Method::DELETE, &elsewhere, None).await.expect("the second chat ends");
        assert!(!docker(&["inspect", &second]).0, "its runner went");
        assert!(inspected(&container)["State"]["Running"].as_bool().unwrap(), "the first goes on");

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
        forget_folder(&session);
    }

    /// Every account's chat in a session works in the session's one folder, at the path each had of its own before:
    /// what one program made, and the conversation it had, is there for the next account's, of the other program too.
    /// What an account kept in the place before sessions had a folder moves in, over nothing. The folder says what it
    /// takes on disk, and stays when the chat, the session and the accounts are gone.
    #[tokio::test]
    #[ignore = "needs the bench stack: scripts/e2e.sh"]
    async fn every_account_s_chat_in_a_session_works_in_its_one_folder_which_stays() {
        let admin = Admin::login().await;
        let first = admin.create(TYPING).await;
        let session =
            admin.call(Method::POST, "/api/sessions", Some(json!({"url": TYPING, "devtools": false}))).await.unwrap();
        let session = session["id"].as_str().unwrap().to_owned();
        admin.adopt(&session);
        let (codex, claude) =
            (account_of(&admin, "folder codex", "codex").await, account_of(&admin, "folder claude", "claude").await);
        let open = |session: &str, account: &str| {
            let (path, body) = (format!("/api/sessions/{session}/chat"), json!({"account": account}));
            let admin = &admin;
            async move { admin.call(Method::POST, &path, Some(body)).await.expect("a chat") }
        };
        let close = |session: &str| {
            let path = format!("/api/sessions/{session}/chat");
            let admin = &admin;
            async move { admin.call(Method::DELETE, &path, None).await.expect("the chat ends") }
        };
        let place = format!("work/{session}");
        // Each account as it was before: with files of its own in the session's place in its home, one by the same name.
        for (account, said) in [(&codex, "codex"), (&claude, "claude")] {
            open(&first, account).await;
            in_chat(
                &first,
                &format!(
                    "mkdir -p ~/{place}/kept && echo {said} > ~/{place}/notes && echo {said} > ~/{place}/kept/{said}"
                ),
            );
            close(&first).await;
        }

        open(&session, &codex).await;
        let moved = common::until!(60, Duration::from_millis(250), {
            let now = in_chat(
                &session,
                &format!("cat ~/{place}/notes ~/{place}/kept/codex 2>/dev/null; ls -A /run/chat/home/{place}"),
            );
            (now == "codex\ncodex\n").then_some(())
        });
        assert!(moved.is_some(), "what the account kept in the place moved into the folder, and nothing stays behind");
        // Its conversations, where its program keeps them and says where each was had, are copied into the folder as they
        // go: the one of this place, and no other.
        let here = format!("/home/runner/{place}");
        in_chat(
            &session,
            &format!(
                "mkdir -p ~/.codex/sessions/2026/01/02 && cd ~/.codex/sessions/2026/01/02 \
             && echo '{{\"type\":\"session_meta\",\"payload\":{{\"cwd\":\"{here}\"}}}}' > rollout-here.jsonl \
             && echo '{{\"type\":\"session_meta\",\"payload\":{{\"cwd\":\"/home/runner/work/000000000000\"}}}}' > rollout-other.jsonl"
            ),
        );
        let kept = format!("~/{place}/.chats/codex-{codex}");
        let copied = common::until!(60, Duration::from_millis(500), {
            let now = in_chat(
                &session,
                &format!("ls {kept} 2>/dev/null; cat ~/{place}/.chats/README.md 2>/dev/null | head -n 1"),
            );
            (now == "rollout-here.jsonl\n# Conversations of this session's chats\n").then_some(())
        });
        assert!(
            copied.is_some(),
            "the conversation had here is in the folder: {}",
            in_chat(&session, &format!("ls -A ~/{place}/.chats {kept}; true"))
        );
        in_chat(&session, "echo '{\"type\":\"more\"}' >> ~/.codex/sessions/2026/01/02/rollout-here.jsonl");
        let grew = common::until!(60, Duration::from_millis(500), {
            let same = in_chat(
                &session,
                &format!(
                    "cmp -s ~/.codex/sessions/2026/01/02/rollout-here.jsonl {kept}/rollout-here.jsonl && echo same; true"
                ),
            );
            (same == "same\n").then_some(())
        });
        assert!(grew.is_some(), "and what the conversation gains is added to its copy");

        // What the folder takes on disk, which grows with what is put in it.
        let size = format!("/api/sessions/{session}/chat/folder");
        let before = admin.get(&size).await.expect("its size")["bytes"].as_u64().expect("a number of bytes");
        in_chat(&session, &format!("head -c 3000000 /dev/urandom > ~/{place}/big"));
        let after = admin.get(&size).await.expect("its size")["bytes"].as_u64().expect("a number of bytes");
        assert!(after >= before + 3_000_000 && after < before + 4_000_000, "{before} then {after}");
        close(&session).await;
        assert_eq!(admin.get(&size).await.expect_err("no chat, no count").0, 404);

        // The other program's account, in the same session: the same folder at the same path, with what the first one
        // left and the conversation it had; its own file by a name the folder has goes beside it, over nothing.
        open(&session, &claude).await;
        let both = common::until!(60, Duration::from_millis(250), {
            let now = in_chat(
                &session,
                &format!(
                    "cd ~/{place} && cat notes from-claude/notes kept/codex from-claude/kept/claude 2>/dev/null; ls -A /run/chat/home/{place}"
                ),
            );
            (now == "codex\nclaude\ncodex\nclaude\n").then_some(())
        });
        assert!(both.is_some(), "{}", in_chat(&session, &format!("ls -AR ~/{place}; true")));
        assert_eq!(
            in_chat(
                &session,
                &format!(
                    "cd ~/{place} && pwd -P && stat -c %s big && head -n 1 .chats/codex-{codex}/rollout-here.jsonl | cut -c1-22"
                )
            ),
            format!("{here}\n3000000\n{{\"type\":\"session_meta\"\n")
        );
        // Its own conversations of this place, as Claude Code keeps them by the folder's path, with what it noted there.
        let project = format!("~/.claude/projects/-home-runner-work-{session}");
        in_chat(
            &session,
            &format!(
                "mkdir -p {project}/memory && echo '{{\"n\":1}}' > {project}/talk.jsonl && echo noted > {project}/memory/MEMORY.md"
            ),
        );
        let kept = format!("~/{place}/.chats/claude-{claude}");
        let copied = common::until!(60, Duration::from_millis(500), {
            let now = in_chat(&session, &format!("cat {kept}/talk.jsonl {kept}/memory/MEMORY.md 2>/dev/null; true"));
            (now == "{\"n\":1}\nnoted\n").then_some(())
        });
        assert!(copied.is_some(), "{}", in_chat(&session, &format!("ls -AR ~/{place}/.chats; true")));

        // The folder outlives the chat, the session, and both accounts.
        admin.clear().await;
        for account in [&codex, &claude] {
            admin.call(Method::DELETE, &format!("/api/chat-accounts/{account}"), None).await.expect("the account goes");
        }
        assert!(docker(&["volume", "inspect", &folder(&session)]).0, "the session's folder stays");
        forget_folder(&session);
        forget_folder(&first);
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
        forget_folder(&session);
    }

    /// What changes reaches the page as it changes, with no asking.
    #[tokio::test]
    #[ignore = "needs the bench stack: scripts/e2e.sh"]
    async fn a_session_and_the_lists_are_told_as_they_change() {
        let admin = Admin::login().await;
        let mut lists = admin.socket("/ws/updates").await;
        let session = std::sync::OnceLock::new();
        // Told as it opens, for what changed before it followed; then as something changes.
        for _ in 0..2 {
            let told = async {
                loop {
                    match lists.next().await {
                        Some(Ok(Message::Text(text))) => return text.to_string(),
                        Some(Ok(_)) => {}
                        other => panic!("{other:?}"),
                    }
                }
            };
            let said = tokio::time::timeout(Duration::from_secs(10), told).await.expect("the lists are told");
            assert_eq!(said, "changed");
            if session.get().is_none() {
                let _ = session.set(admin.create(TYPING).await);
            }
        }
        let session = session.get().expect("started").clone();

        // A viewer is told at once: nothing open, no chat.
        let viewer = admin.watch(&session, 0).await;
        let state = viewer.wait_for("the session's state", |seen| seen.session_state.clone()).await;
        assert!(state["help"].is_null() && state["chat"].is_null() && state["asks"] == json!([]), "{state}");

        // The agent asks for a person: told; given back: told it closed, as the lease moves.
        let asked = json!({"reason": "login", "message": "Sign in"});
        admin.agent(&session, "request_person", asked).await.expect("the request");
        let epoch = viewer
            .wait_for("the request", |seen| {
                let open = seen.session_state.as_ref().is_some_and(|s| s["help"]["reason"] == "login");
                open.then_some(())
            })
            .await;
        let () = epoch;
        admin.take(&session, &viewer).await;
        admin.hand_back(&session, &viewer).await;
        viewer
            .wait_for("the request closed", |seen| {
                seen.session_state.as_ref().is_some_and(|s| s["help"].is_null()).then_some(())
            })
            .await;

        // What the containers use: only while asked for.
        viewer.send(display_protocol::Message::UsageWatch { on: true }).await;
        let usage = viewer.wait_for("the usage", |seen| seen.usage.clone()).await;
        assert!(usage["containers"]["containers"]["browser"].is_object() && usage["viewers"] == 1, "{usage}");
        viewer.send(display_protocol::Message::UsageWatch { on: false }).await;
        admin.clear().await;
    }
}
