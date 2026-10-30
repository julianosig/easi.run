//! Persistent sessions through the running admin application: a login a page kept (a cookie) carries over from one
//! session to the next through a saved session (a saved Chrome profile, which the administrator's API calls a profile),
//! one running session at a time resumes it, a token it does not allow learns of no saved session, and one it allows
//! reaches the run the person resumed.

mod common;

use common::{Admin, Agent, Refused};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

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
    let first = start(&admin, &id, &format!("{PAGE}?set={login}")).await;
    assert_eq!(listed(&admin, &id).await["session"], first, "the profile is in use");
    let second = admin.call(Method::POST, "/api/sessions", Some(json!({"url": PAGE, "profile": id}))).await;
    assert!(matches!(&second, Err(Refused(409, why)) if why.contains("already running")), "{second:?}");
    let stopped = admin.call(Method::DELETE, &format!("/api/sessions/{first}"), None).await;
    assert!(stopped.is_ok(), "{stopped:?}");
    let saved = listed(&admin, &id).await;
    assert!(saved["used"].is_u64() && saved["size"].as_u64() > Some(0), "saved at the stop: {saved}");
    assert_eq!((&saved["session"], &saved["unsaved"]), (&Value::Null, &Value::Null), "{saved}");

    // A session from it again, at the page without the login: the cookie came with the profile.
    let again = start(&admin, &id, PAGE).await;
    let text = common::page_text(&admin, &again).await;
    assert!(text.contains(&format!("login: {login}")), "{text}");
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
    assert_eq!(listed["saved_sessions"], json!([{"id": id, "name": NAME, "running": false}]), "{listed}");

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
    assert_eq!(resumed["saved_session"], json!({"id": id, "name": NAME}), "{resumed}");
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
