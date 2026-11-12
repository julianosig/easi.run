//! Persistent sessions through the running admin application: a login a page kept (a cookie) carries over from one
//! session to the next through a saved session (a saved Chrome profile, which the administrator's API calls a profile),
//! one running session at a time resumes it, a token it does not allow learns of no saved session, and one it allows
//! reaches the run the person resumed. Stopping the admin, as an update does, keeps every session that runs: saved, and
//! started again once the password is back.

mod common;

use common::{Admin, Agent, Refused, until};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use std::time::Duration;

const PAGE: &str = "https://fixture.bench/cookie.html";
/// The profile and the token this test creates are named so, and an earlier run's are removed first.
const NAME: &str = "e2e profiles";

/// Removes every profile and token named [`NAME`]: an earlier run's, or this one's.
async fn remove(admin: &Admin) {
    admin.clear().await;
    for kind in ["profiles", "tokens"] {
        admin.remove_named(kind, |n| n == NAME).await;
    }
}

/// The profile `id` as the administrator lists it.
async fn listed(admin: &Admin, id: &str) -> Value {
    let profiles = admin.get("/api/profiles").await.expect("the profiles");
    profiles["profiles"].as_array().unwrap().iter().find(|p| p["id"] == id).expect("the profile listed").clone()
}

/// A session from the profile `id` at `url`, once its page has loaded and found the login: a page that loaded without
/// it fails at once, apart from one that never loaded.
async fn start(admin: &Admin, id: &str, url: &str) -> String {
    let session = admin.create_with(json!({"url": url, "profile": id})).await;
    let loaded = |windows: &[Value]| {
        let title = windows.iter().find_map(|w| w["title"].as_str().filter(|t| t.starts_with("Cookie ")))?;
        (title.starts_with("Cookie set - ") || title.starts_with("Cookie none - ")).then(|| title.to_owned())
    };
    let title = admin.wait_windows(&session, "the cookie page", loaded).await;
    assert!(title.starts_with("Cookie set - "), "the page found no login: {title}");
    session
}

/// A token's resumption of the saved session `id`: the agent API's status and answer.
async fn start_as(agent: &Agent, id: &str) -> (StatusCode, Value) {
    agent.rest(Method::POST, "/api/agent/sessions", Some(json!({"url": PAGE, "resume": id}))).await
}

async fn carried_over(admin: Admin, id: String, agent: Agent) {
    let login = format!("e2e-{}", std::process::id());
    let first = start(&admin, &id, &format!("{PAGE}?set={login}#first")).await;
    assert_eq!(listed(&admin, &id).await["session"], first, "the profile is in use");
    let second = admin.call(Method::POST, "/api/sessions", Some(json!({"url": PAGE, "profile": id}))).await;
    assert!(matches!(&second, Err(Refused(409, why)) if why.contains("already running")), "{second:?}");
    // Another session on the same site meanwhile, of another account say, shares none of it: its browser is its own.
    // Started beside it (create_with would stop the first): stopped below.
    let other = admin.call(Method::POST, "/api/sessions", Some(json!({"url": PAGE}))).await.expect("another session");
    let other = other["id"].as_str().expect("an id").to_owned();
    let other_found = |windows: &[Value]| {
        let title = windows.iter().find_map(|w| w["title"].as_str())?;
        (title.starts_with("Cookie set - ") || title.starts_with("Cookie none - ")).then(|| title.to_owned())
    };
    let title = admin.wait_windows(&other, "the cookie page", other_found).await;
    assert!(title.starts_with("Cookie none - "), "another session found this one's login: {title}");
    admin.stop(&other).await;
    let stopped = admin.call(Method::DELETE, &format!("/api/sessions/{first}"), None).await;
    assert!(stopped.is_ok(), "{stopped:?}");
    let saved = listed(&admin, &id).await;
    assert!(saved["used"].is_u64() && saved["size"].as_u64() > Some(0), "saved at the stop: {saved}");
    assert_eq!((&saved["session"], &saved["unsaved"]), (&Value::Null, &Value::Null), "{saved}");

    // A session from it again reopens the page it had open (not the one asked for), which finds the login only in
    // its cookie: the cookie came with the profile.
    let again = start(&admin, &id, &format!("{PAGE}#again")).await;
    // A restored tab keeps its saved title while it loads again: polled until the page itself says so.
    let expected = format!("login: {login}, at: #first");
    let text = until!(40, Duration::from_millis(500), {
        let text = common::page_text(&admin, &again).await;
        text.contains(&expected).then_some(text)
    });
    assert!(text.is_some(), "the page it had open, with its login: {}", common::page_text(&admin, &again).await);
    admin.stop(&again).await;

    // A token the saved session does not allow: no such saved session, as for an id that is none's.
    let (status, listed) = agent.rest(Method::GET, "/api/agent/saved-sessions", None).await;
    assert_eq!((status, &listed["saved_sessions"]), (StatusCode::OK, &json!([])), "{listed}");
    let (status, refused) = start_as(&agent, &id).await;
    assert_eq!((status, &refused["error"]), (StatusCode::NOT_FOUND, &json!("no such saved session")), "{refused}");
    assert_eq!(start_as(&agent, "0123456789abcdef").await, (status, refused));

    // Allowed, the token lists it and resumes it, and the session it starts names the saved session it resumed.
    let allowed = admin.call(Method::PUT, &format!("/api/profiles/{id}"), Some(json!({"allowed": [agent.id]}))).await;
    assert!(allowed.is_ok(), "{allowed:?}");
    let (_, listed) = agent.rest(Method::GET, "/api/agent/saved-sessions", None).await;
    let listed = &listed["saved_sessions"][0];
    assert_eq!(
        (&listed["id"], &listed["name"], &listed["running"]),
        (&json!(id), &json!(NAME), &json!(false)),
        "{listed}"
    );
    assert!(
        listed["saved"].is_u64() && listed["problem"].is_null(),
        "saved by the person, with nothing wrong: {listed}"
    );

    // The person resumes it from the admin page: the token reaches that run, and its stop saves it.
    let theirs = start(&admin, &id, PAGE).await;
    let (_, sessions) = agent.rest(Method::GET, "/api/agent/sessions", None).await;
    let ids: Vec<&Value> = sessions["sessions"].as_array().expect("the sessions").iter().map(|s| &s["id"]).collect();
    assert_eq!(ids, [&json!(theirs)], "{sessions}");
    let (status, _) = agent.rest(Method::DELETE, &format!("/api/agent/sessions/{theirs}"), None).await;
    assert_eq!(status, StatusCode::OK);
    let saved = crate::listed(&admin, &id).await;
    assert_eq!((&saved["session"], &saved["unsaved"]), (&Value::Null, &Value::Null), "{saved}");

    let (status, resumed) = start_as(&agent, &id).await;
    assert_eq!(status, StatusCode::OK, "{resumed}");
    let session = resumed["id"].as_str().expect("its id").to_owned();
    admin.adopt(&session);
    let resumed_from = &resumed["saved_session"];
    assert_eq!((&resumed_from["id"], &resumed_from["name"]), (&json!(id), &json!(NAME)), "{resumed}");
    assert!(resumed_from["saved"].is_u64() && resumed_from["problem"].is_null(), "{resumed}");
    assert!(resumed.get("profile").is_none(), "{resumed}");
    let (_, listed) = agent.rest(Method::GET, "/api/agent/saved-sessions", None).await;
    assert_eq!(listed["saved_sessions"][0]["running"], true, "{listed}");
    let (status, _) = agent.rest(Method::DELETE, &format!("/api/agent/sessions/{session}"), None).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_login_carries_over_through_a_saved_session_and_a_token_resumes_only_one_it_is_allowed() {
    let admin = Admin::login().await;
    remove(&admin).await;
    let profile = admin.call(Method::POST, "/api/profiles", Some(json!({"name": NAME}))).await.expect("a profile");
    let id = profile["id"].as_str().expect("its id").to_owned();
    let agent = admin.token(NAME).await;
    // Its own task, so the profile and the token are removed whatever it finds.
    let outcome = tokio::spawn(carried_over(Admin::login().await, id, agent)).await;
    remove(&admin).await;
    if let Err(failed) = outcome {
        std::panic::resume_unwind(failed.into_panic());
    }
}

/// A session stopped while its agent's call runs: the call ends first, so nothing acts on the page while it is saved.
async fn stopped_during_a_call(admin: &Admin, id: &str) {
    let session = admin.create_with(json!({"url": PAGE, "profile": id})).await;
    admin.wait_windows(&session, "the page", common::first_window).await;
    let caller = Admin::login().await;
    let called = session.clone();
    let call = tokio::spawn(async move {
        let script = json!({"script": "await cua.sleep(90000); return 'slept';", "timeout_ms": 120000});
        caller.agent(&called, "run_script", script).await
    });
    tokio::time::sleep(Duration::from_secs(2)).await;
    let started = std::time::Instant::now();
    admin.stop(&session).await;
    assert!(started.elapsed() < Duration::from_secs(60), "the stop waited for the call: {:?}", started.elapsed());
    assert!(call.await.expect("the call's task").is_err(), "the call ran to its end");
    let saved = listed(admin, id).await;
    assert!(saved["unsaved"].is_null() && !saved["used"].is_null(), "not saved: {saved}");
}

#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_call_under_way_ends_before_a_stopped_session_saves_its_profile() {
    let admin = Admin::login().await;
    let profile =
        admin.call(Method::POST, "/api/profiles", Some(json!({"name": "e2e quiet save"}))).await.expect("a profile");
    let id = profile["id"].as_str().expect("its id").to_owned();
    let outcome = tokio::spawn({
        let id = id.clone();
        async move { stopped_during_a_call(&Admin::login().await, &id).await }
    })
    .await;
    let path = format!("/api/profiles/{id}");
    admin.call(Method::DELETE, &path, None).await.expect("deleted");
    if let Err(failed) = outcome {
        std::panic::resume_unwind(failed.into_panic());
    }
}

mod exclusive {
    use super::*;

    /// The bench admin's container, which this test restarts: no other test may run meanwhile.
    const ADMIN: &str = "easi-run-bench-admin-1";

    /// Stops the admin as an update does (compose.yaml's grace period) and waits until it answers again.
    async fn restart() {
        let (restarted, said) = common::docker(&["restart", "--time", "120", ADMIN]);
        assert!(restarted, "{said}");
        assert!(
            until!(120, Duration::from_secs(1), common::answers().await.then_some(())).is_some(),
            "the admin is back"
        );
    }

    /// The page of `session`, once it shows `expected`.
    async fn shows(admin: &Admin, session: &str, expected: &str) {
        let text = until!(60, Duration::from_millis(500), {
            let text = common::page_text(admin, session).await;
            text.contains(expected).then_some(text)
        });
        assert!(text.is_some(), "expected {expected:?}: {}", common::page_text(admin, session).await);
    }

    #[tokio::test]
    #[ignore = "needs the bench stack: scripts/e2e.sh"]
    async fn a_persistent_session_that_runs_as_the_admin_stops_is_saved_and_starts_again() {
        let admin = Admin::login().await;
        remove(&admin).await;
        let profile = admin.call(Method::POST, "/api/profiles", Some(json!({"name": NAME}))).await.expect("a profile");
        let id = profile["id"].as_str().expect("its id").to_owned();
        let login = format!("e2e-restart-{}", std::process::id());
        let before = start(&admin, &id, &format!("{PAGE}?set={login}#before")).await;

        restart().await;
        // The password opens what was kept: the session starts again from its saved session, by itself.
        let admin = Admin::login().await;
        let again = admin.resumed().await;
        let again = again.iter().find(|s| s["profile"]["id"] == json!(id)).expect("it started again");
        assert_eq!(again["was"], json!([before]), "{again}");
        let saved = listed(&admin, &id).await;
        assert!(saved["used"].is_u64() && saved["size"].as_u64() > Some(0), "saved as the admin stopped: {saved}");
        // The page it had open finds the login it made in its cookie.
        shows(&admin, again["id"].as_str().unwrap(), &format!("login: {login}, at: #before")).await;
        admin.clear().await;
        remove(&admin).await;
    }

    /// A session that runs from no saved one comes back too: its browser as it was (its login, and the page it had
    /// gone to, not the one it started at), for the token that started it, which is told what became of the id it
    /// holds, before the password and after, and that its pages were loaded again.
    #[tokio::test]
    #[ignore = "needs the bench stack: scripts/e2e.sh"]
    async fn a_session_that_runs_as_the_admin_stops_starts_again_once_the_password_is_back() {
        let admin = Admin::login().await;
        remove(&admin).await;
        let agent = admin.token(NAME).await;
        let login = format!("e2e-again-{}", std::process::id());
        let url = format!("{PAGE}?set={login}#before");
        let (status, started) = agent.rest(Method::POST, "/api/agent/sessions", Some(json!({"url": url}))).await;
        assert_eq!(status, StatusCode::OK, "{started}");
        let before = started["id"].as_str().expect("an id").to_owned();
        admin.adopt(&before);
        shows(&admin, &before, &format!("login: {login}, at: #before")).await;
        // It goes on to another page: what comes back is where it was, not where it started.
        let (pid, window) = admin.wait_windows(&before, "the page's window", common::first_window).await;
        let keys = json!({"pid": pid, "window_id": window, "keys": ["ctrl", "l"]});
        admin.agent(&before, "hotkey", keys).await.expect("the address bar");
        let address = json!({"pid": pid, "window_id": window, "text": format!("{PAGE}?page=2#moved\n")});
        admin.agent(&before, "type_text", address).await.expect("an address typed");
        shows(&admin, &before, &format!("login: {login}, at: #moved")).await;

        restart().await;
        // Before the password, the token learns that its session waits for it.
        let path = format!("/api/agent/sessions/{before}");
        let (status, waits) = agent.rest(Method::GET, &path, None).await;
        assert_eq!((status, &waits["code"]), (StatusCode::CONFLICT, &json!("session-waits")), "{waits}");

        let admin = Admin::login().await;
        let again = admin.resumed().await;
        let again = again.iter().find(|s| s["was"] == json!([before])).expect("it started again");
        let now = again["id"].as_str().expect("an id").to_owned();
        assert!(again["profile"].is_null(), "it holds no saved session: {again}");
        // The id it had says which session it is now, to the token that reaches it.
        let (status, moved) = agent.rest(Method::GET, &path, None).await;
        assert_eq!((status, &moved["code"]), (StatusCode::CONFLICT, &json!("session-resumed")), "{moved}");
        assert_eq!(moved["session"], json!(now), "{moved}");
        let (status, mine) = agent.rest(Method::GET, &format!("/api/agent/sessions/{now}"), None).await;
        assert_eq!(status, StatusCode::OK, "the token's own again: {mine}");
        // Its agent is told once that its pages were loaded again, and its timeline says it started again.
        let note = "this session was started again";
        let first = admin.agent(&now, "list_windows", json!({})).await.expect("the windows");
        assert!(first.to_string().contains(note), "{first}");
        let second = admin.agent(&now, "list_windows", json!({})).await.expect("the windows");
        assert!(!second.to_string().contains(note), "told once: {second}");
        let timeline = admin.get(&format!("/api/sessions/{now}/timeline")).await.expect("the timeline");
        let events = timeline["events"].as_array().expect("its events");
        assert!(events.iter().any(|e| e["kind"] == "session" && e["state"] == "resumed"), "{timeline}");
        // Its login, on the page it had gone to.
        shows(&admin, &now, &format!("login: {login}, at: #moved")).await;

        // Stopped as any session, it leaves nothing to start again.
        admin.stop(&now).await;
        restart().await;
        let admin = Admin::login().await;
        assert!(admin.resumed().await.is_empty(), "a session stopped before the admin was starts no more");
        remove(&admin).await;
    }
}
