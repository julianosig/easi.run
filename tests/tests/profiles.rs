//! Saved profiles through the running admin application: a login a page kept (a cookie) carries over from one session to
//! the next through a saved profile, one session at a time uses it, and a token the profile does not allow learns of no
//! profile.

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

/// A session from the profile `id` at `url`, once its page has loaded.
async fn start(admin: &Admin, id: &str, url: &str) -> String {
    let session = admin.create_with(json!({"url": url, "profile": id})).await;
    admin.wait_title(&session, "Cookie set").await;
    session
}

/// A token's start of a session from `profile`: the agent API's status and answer.
async fn start_as(agent: &Agent, profile: &str) -> (StatusCode, Value) {
    agent.rest(Method::POST, "/api/agent/sessions", Some(json!({"url": PAGE, "profile": profile}))).await
}

async fn carried_over(admin: Admin, id: String, agent: Agent) {
    let login = format!("e2e-{}", std::process::id());
    let first = start(&admin, &id, &format!("{PAGE}?set={login}")).await;
    assert_eq!(listed(&admin, &id).await["session"], first, "the profile is in use");
    let second = admin.call(Method::POST, "/api/sessions", Some(json!({"url": PAGE, "profile": id}))).await;
    assert!(matches!(&second, Err(Refused(409, why)) if why.contains("uses this profile")), "{second:?}");
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

    // A token the profile does not allow: no such profile, as for an id that is no profile's.
    let (status, refused) = start_as(&agent, &id).await;
    assert_eq!((status, &refused["error"]), (StatusCode::NOT_FOUND, &json!("no such profile")), "{refused}");
    assert_eq!(start_as(&agent, "0123456789abcdef").await, (status, refused));
}

#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_login_carries_over_through_a_saved_profile_and_a_token_not_allowed_learns_of_none() {
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
