//! What an installation holds outlasts an update. Run by `scripts/update-test.sh` alone, against a stack of its own
//! that starts as the release `install.sh` pins (its published images, its Compose file) and becomes this checkout's
//! when the test runs `UPDATE_COMMAND`, which stops the admin and starts the new one as `install.sh --update` does.
//! Until then the test speaks to the release before this one: it asks only what that release answers.

mod common;

use common::{Admin, FIELD, until};
use display_protocol::{LeaseHolder, Message};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use std::time::Duration;

/// A page that keeps a login in a cookie and says which it found (bench/fixtures/cookie.html).
const PAGE: &str = "https://fixture.bench/cookie.html";
/// What this test names the token it makes, and starts the names of its saved sessions with.
const NAME: &str = "update";
/// The bench stack's test upstream (compose.bench.yaml), which an egress leaves through.
const UPSTREAM: &str = "http://10.250.250.20:3128";
/// What a person types while holding control: nothing an agent reads may show it, before the update or after.
const TYPED: &str = "updatesecret";

/// A session beside the others this login started (`Admin::create_with` would stop them first), counted as its own.
async fn session(admin: &Admin, body: Value) -> String {
    let started = admin.call(Method::POST, "/api/sessions", Some(body)).await.expect("a session");
    let id = started["id"].as_str().expect("an id").to_owned();
    admin.adopt(&id);
    id
}

/// The page of `session`, once it shows `expected`.
async fn shows(admin: &Admin, session: &str, expected: &str) {
    let text = until!(120, Duration::from_millis(500), {
        let text = common::page_text(admin, session).await;
        text.contains(expected).then_some(text)
    });
    assert!(text.is_some(), "expected {expected:?}: {}", common::page_text(admin, session).await);
}

/// The agent types `address` into the address bar of `session`'s window: the page goes there.
async fn go(admin: &Admin, session: &str, address: &str) {
    let (pid, window) = admin.wait_windows(session, "the page's window", common::first_window).await;
    let keys = json!({"pid": pid, "window_id": window, "keys": ["ctrl", "l"]});
    admin.agent(session, "hotkey", keys).await.expect("the address bar");
    let typed = json!({"pid": pid, "window_id": window, "text": format!("{address}\n")});
    admin.agent(session, "type_text", typed).await.expect("an address typed");
}

/// A new saved session named after `what`, by its id.
async fn saved(admin: &Admin, what: &str) -> String {
    let name = json!({"name": format!("{NAME} {what}")});
    let profile = admin.call(Method::POST, "/api/profiles", Some(name)).await.expect("a saved session");
    profile["id"].as_str().expect("its id").to_owned()
}

/// Signs in with the password, then `code` for the second step it asks: the login, or the status that refused the
/// code. The admin holds sign-ins back for a few seconds after ones it refused (429): those are waited out.
async fn sign_in(admin: &Admin, code: &str) -> Result<Admin, StatusCode> {
    let password = std::env::var("ADMIN_PASSWORD").expect("ADMIN_PASSWORD");
    let post = |path: &str, body: Value| {
        admin.http.post(format!("{}{path}", admin.origin)).header("origin", &admin.origin).json(&body).send()
    };
    for _ in 0..30 {
        let answer = post("/api/login", json!({"password": password})).await.expect("the password's answer");
        if answer.status() == StatusCode::TOO_MANY_REQUESTS {
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        }
        assert!(answer.status().is_success() && answer.headers().get("set-cookie").is_none(), "the password alone");
        let answer: Value = answer.json().await.expect("its json");
        let step = answer["second_step"]["step"].as_str().expect("a pending step").to_owned();
        let signed_in = post("/api/login/code", json!({"step": step, "code": code})).await.expect("the code's answer");
        if signed_in.status() == StatusCode::TOO_MANY_REQUESTS {
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        }
        if !signed_in.status().is_success() {
            return Err(signed_in.status());
        }
        let cookie = common::session_cookie(&signed_in).expect("the login's cookie");
        let csrf = signed_in.json::<Value>().await.expect("its json")["csrf"].as_str().expect("a csrf").to_owned();
        return Ok(admin.with_login(cookie, csrf));
    }
    panic!("sign-ins were held back for a minute")
}

/// Signs in with `passkey` alone, as the login screen does, with the assertion `signed` makes of the admin's options:
/// the login, or the status that refused it. Sign-ins held back (429) are waited out.
async fn passkey_sign_in(
    admin: &Admin,
    passkey: &mut common::Passkey,
    signed: impl Fn(&mut common::Passkey, &Value) -> Value,
) -> Result<Admin, StatusCode> {
    let post = |path: &str, body: Value| {
        admin.http.post(format!("{}{path}", admin.origin)).header("origin", &admin.origin).json(&body).send()
    };
    for _ in 0..30 {
        let begun = post("/api/login/passkey", Value::Null).await.expect("the passkey's ceremony");
        if begun.status() == StatusCode::TOO_MANY_REQUESTS {
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        }
        assert!(begun.status().is_success(), "no passkey ceremony: {}", begun.status());
        let begun: Value = begun.json().await.expect("its json");
        let answer = json!({"ceremony": begun["ceremony"], "credential": signed(passkey, &begun["options"])});
        let finished = post("/api/login/passkey/finish", answer).await.expect("the passkey's answer");
        if finished.status() == StatusCode::TOO_MANY_REQUESTS {
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        }
        if !finished.status().is_success() {
            return Err(finished.status());
        }
        let cookie = common::session_cookie(&finished).expect("the login's cookie");
        let csrf = finished.json::<Value>().await.expect("its json")["csrf"].as_str().expect("a csrf").to_owned();
        return Ok(admin.with_login(cookie, csrf));
    }
    panic!("sign-ins were held back for a minute")
}

/// What `command` prints in the chat's container of `session`, in this installation (scripts/update-test.sh names
/// its controller's instance).
fn in_runner(session: &str, command: &str) -> Option<String> {
    let instance = std::env::var("UPDATE_INSTANCE").unwrap_or_else(|_| "easirun-update".into());
    let (ok, said) = common::docker(&["exec", &format!("{instance}-{session}-runner"), "sh", "-c", command]);
    ok.then_some(said)
}

/// Replaces the release that runs by this checkout's, as an update does, and waits until the new admin answers.
async fn update() {
    let command = std::env::var("UPDATE_COMMAND").expect("UPDATE_COMMAND: scripts/update-test.sh sets it");
    let done = tokio::task::spawn_blocking(move || std::process::Command::new(command).status()).await.unwrap();
    assert!(done.is_ok_and(|status| status.success()), "the update's command failed");
    assert!(until!(180, Duration::from_secs(1), common::answers().await.then_some(())).is_some(), "the admin is back");
}

#[tokio::test]
#[ignore = "needs a stack of its own: scripts/update-test.sh"]
async fn what_an_installation_holds_outlasts_an_update() {
    let admin = Admin::login().await;

    // A saved session nobody runs as the update comes: a login made in it, kept as it stopped.
    let rested = saved(&admin, "rested").await;
    let first = session(&admin, json!({"url": format!("{PAGE}?set=rested#first"), "profile": rested})).await;
    shows(&admin, &first, "login: rested, at: #first").await;
    admin.stop(&first).await;

    // A session whose replay and timeline were kept as it stopped.
    let recorded = session(&admin, json!({"url": common::TYPING, "devtools": false})).await;
    common::page_text(&admin, &recorded).await;
    let replay = format!("/api/sessions/{recorded}/replay");
    until!(80, Duration::from_millis(500), {
        admin.get(&replay).await.ok().filter(|i| i["segments"].as_array().is_some_and(|s| !s.is_empty()))
    })
    .expect("a segment recorded");
    admin.stop(&recorded).await;

    // Two that run as the update comes, one from a saved session and one a token's own. Each has gone on from the
    // address that set its login to one that sets none: started again without what its browser kept, it would open
    // where it started, and that page would set the login anew.
    let kept = saved(&admin, "kept").await;
    let ran = session(&admin, json!({"url": format!("{PAGE}?set=kept#before"), "profile": kept})).await;
    shows(&admin, &ran, "login: kept, at: #before").await;
    go(&admin, &ran, &format!("{PAGE}?page=2#moved")).await;
    shows(&admin, &ran, "login: kept, at: #moved").await;
    // A chat open in it, with an account whose home holds a file.
    let account = json!({"name": NAME, "product": "codex"});
    let account = admin.call(Method::POST, "/api/chat-accounts", Some(account)).await.expect("a chat account");
    let account = account["id"].as_str().expect("its id").to_owned();
    let chat = json!({"account": account});
    admin.call(Method::POST, &format!("/api/sessions/{ran}/chat"), Some(chat)).await.expect("a chat");
    let wrote = until!(40, Duration::from_millis(500), in_runner(&ran, "echo kept > /home/runner/update-kept"));
    assert!(wrote.is_some(), "the chat's home took no file");
    // And a file where its program works, which the release before keeps in the account's home.
    let worked = "mkdir -p /home/runner/work/$RUNNER_PLACE && echo kept > /home/runner/work/$RUNNER_PLACE/work-kept";
    assert!(in_runner(&ran, worked).is_some(), "the chat's place took no file");
    let agent = admin.token(NAME).await;
    let url = format!("{PAGE}?set=token#before");
    let (status, started) = agent.rest(Method::POST, "/api/agent/sessions", Some(json!({"url": url}))).await;
    assert_eq!(status, StatusCode::OK, "{started}");
    let held = started["id"].as_str().expect("an id").to_owned();
    admin.adopt(&held);
    shows(&admin, &held, "login: token, at: #before").await;
    go(&admin, &held, &format!("{PAGE}?page=2#moved")).await;
    shows(&admin, &held, "login: token, at: #moved").await;

    // The token may use the saved session that rests; another token is revoked.
    let grant = json!({"allowed": [agent.id]});
    admin.call(Method::PUT, &format!("/api/profiles/{rested}"), Some(grant)).await.expect("the token allowed");
    let revoked = admin.token(&format!("{NAME} revoked")).await;
    admin.call(Method::DELETE, &format!("/api/tokens/{}", revoked.id), None).await.expect("a token revoked");
    assert_eq!(revoked.rest(Method::GET, "/api/agent/tools", None).await.0, StatusCode::UNAUTHORIZED);

    // An egress, with the place it says its sessions are in.
    let place = json!({"name": NAME, "proxy": UPSTREAM, "timezone": "Europe/Berlin", "languages": ["de-DE", "de"]});
    let egress = admin.egress(place).await.expect("an egress whose check passes");
    let egress = egress["id"].as_str().expect("its id").to_owned();

    // A third session that runs, where a person typed something while holding control: the agent reads it redacted,
    // in the field and wherever a page shows it after (here, as the login a page keeps). It needs a viewer the
    // release before takes (scripts/update-test.sh says when it does not).
    let mut typed = None;
    if std::env::var("UPDATE_VIEWER").as_deref() != Ok("0") {
        let session = session(&admin, json!({"url": common::TYPING, "devtools": false})).await;
        common::page_text(&admin, &session).await;
        let person = admin.watch(&session, 0).await;
        person.wait_lease(LeaseHolder::Agent).await;
        let epoch = admin.take(&session, &person).await;
        person.click(epoch, FIELD.0, FIELD.1).await;
        person.type_keys(epoch, TYPED).await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        admin.hand_back(&session, &person).await;
        person.close().await;
        go(&admin, &session, &format!("{PAGE}?set={TYPED}#typed")).await;
        shows(&admin, &session, "login: [redacted], at: #typed").await;
        typed = Some(session);
    }

    // The second step at sign-in, last: every login after it needs a code.
    let drawn = admin.call(Method::POST, "/api/security/app", None).await.expect("an authenticator's secret");
    let secret = common::base32(drawn["secret"].as_str().expect("the secret"));
    let confirm = json!({"code": common::app_code(&secret, 0)});
    let confirmed = admin.call(Method::POST, "/api/security/app/confirm", Some(confirm)).await.expect("the app on");
    let recovery: Vec<String> =
        confirmed["recovery_codes"].as_array().expect("codes").iter().map(|c| c.as_str().unwrap().to_owned()).collect();
    // A recovery code is used before the update: it signs in no more after it.
    sign_in(&admin, &recovery[0]).await.expect("a recovery code signs in");
    // A passkey, which signs in by itself: once before the update, so its counter has moved on.
    let password = std::env::var("ADMIN_PASSWORD").expect("ADMIN_PASSWORD");
    let mut passkey = common::Passkey::new(&admin.origin);
    let begun = json!({"password": password});
    let begun = admin.call(Method::POST, "/api/security/passkeys", Some(begun)).await.expect("a passkey begun");
    let added = json!({"credential": passkey.registration(&begun["options"])});
    admin.call(Method::POST, "/api/security/passkeys/finish", Some(added)).await.expect("the passkey added");
    passkey_sign_in(&admin, &mut passkey, |key, options| key.assertion(options)).await.expect("the passkey signs in");
    // A code of the app's is used too, last and at the start of a step: the admin takes the code of the step after
    // now's, and would take that code again until the step after its own is over, a minute and a half from here,
    // which the update must fit in for its refusal to say that its use was kept.
    while std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() % 30 >= 3 {
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let used_step = common::app_step() + 1;
    let used = common::app_code_at(&secret, used_step);
    sign_in(&admin, &used).await.expect("the app's next code signs in");

    let identity = common::certificate().await;

    update().await;

    // The admin is who it was to a browser that accepted its certificate.
    assert!(common::certificate().await == identity, "the admin shows another certificate after the update");
    // The password it had opens the new release only with the second step it had: a recovery code, once, and the
    // authenticator's code.
    let unsigned = admin;
    let refused = Some(StatusCode::UNAUTHORIZED);
    // First, while the admin would otherwise still take it: the app's code used before the update is refused. That
    // says its use was kept only if the whole try was over before the code aged out (the step after its own): an
    // update may rightly take longer than that minute and a half, and then this says so, where the run shows it.
    let again = sign_in(&unsigned, &used).await.err();
    let tried_by = common::app_step();
    assert_eq!(again, refused, "a code of the app's used before the update");
    if tried_by > used_step + 1 {
        println!("update: NOT PROVEN this run: the app's used code had aged out before it was tried again");
    }
    // The passkey's counter is the one it reached: an answer that counts no further is a copy's, and refused; the
    // next one signs in.
    let copied = passkey_sign_in(&unsigned, &mut passkey, |key, options| {
        key.counter -= 1;
        key.assertion(options)
    });
    assert_eq!(copied.await.err(), refused, "a passkey's answer with the count it had before the update");
    passkey_sign_in(&unsigned, &mut passkey, |key, options| key.assertion(options))
        .await
        .expect("the passkey signs in");
    assert_eq!(sign_in(&unsigned, &recovery[0]).await.err(), refused, "a recovery code used before the update");
    sign_in(&unsigned, &recovery[1]).await.expect("another recovery code signs in");
    assert_eq!(sign_in(&unsigned, &recovery[1]).await.err(), refused, "once only");
    // The app's code of a step after the one used signs in.
    while common::app_step() <= used_step {
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    let admin = sign_in(&unsigned, &common::app_code(&secret, 0)).await.expect("the authenticator's code signs in");
    // What was kept with the password is there.
    let again = admin.resumed().await;
    let of_saved = again.iter().find(|s| s["profile"]["id"] == json!(kept)).expect("the saved one started again");
    assert_eq!(of_saved["was"], json!([ran]), "{of_saved}");
    let of_saved_id = of_saved["id"].as_str().expect("an id").to_owned();
    shows(&admin, &of_saved_id, "login: kept, at: #moved").await;
    // Its chat is open again with its account, whose home still holds the file.
    assert_eq!(of_saved["chat"]["account"], json!(account), "its chat, with its account: {of_saved}");
    let kept = until!(60, Duration::from_millis(500), in_runner(&of_saved_id, "cat /home/runner/update-kept"));
    assert_eq!(kept.as_deref().map(str::trim), Some("kept"), "the chat account's home lost its file");
    // What it kept where its program works is in the session's folder now, the one of every account that chats in
    // it, at the path it had; nothing of it stays behind in the home.
    let moved = "cat /home/runner/work/$RUNNER_PLACE/work-kept && ls -A /run/chat/home/work/$RUNNER_PLACE";
    let worked = until!(60, Duration::from_millis(500), in_runner(&of_saved_id, moved).filter(|said| said == "kept\n"));
    assert!(
        worked.is_some(),
        "the chat's place lost its file: {:?}",
        in_runner(&of_saved_id, "ls -AR /home/runner/work /run/chat/home/work")
    );
    let of_token = again.iter().find(|s| s["was"] == json!([held])).expect("the token's started again");
    let now = of_token["id"].as_str().expect("an id").to_owned();
    shows(&admin, &now, "login: token, at: #moved").await;
    // The token is still the one that holds it, and learns where its session went.
    let (status, moved) = agent.rest(Method::GET, &format!("/api/agent/sessions/{held}"), None).await;
    assert_eq!((status, &moved["code"]), (StatusCode::CONFLICT, &json!("session-resumed")), "{moved}");
    assert_eq!(moved["session"], json!(now), "{moved}");
    let (status, mine) = agent.rest(Method::GET, &format!("/api/agent/sessions/{now}"), None).await;
    assert_eq!(status, StatusCode::OK, "the token's own again: {mine}");

    // What the person typed stays redacted for the agent in the session that started again.
    if let Some(typed) = typed {
        let of_typed = again.iter().find(|s| s["was"] == json!([typed])).expect("the third started again");
        let of_typed = of_typed["id"].as_str().expect("an id").to_owned();
        shows(&admin, &of_typed, "login: [redacted], at: #typed").await;
        assert!(!common::page_text(&admin, &of_typed).await.contains(TYPED), "what the person typed reached the agent");
        // The administrator's agent call naming its old id learns where it went, as a call refused while the admin
        // stopped is told to ask.
        match admin.agent(&typed, "list_windows", json!({})).await {
            Err(common::Refused(409, said)) => {
                assert!(said.contains("session-resumed") && said.contains(&of_typed), "{said}")
            }
            other => panic!("the old id: {other:?}"),
        }
    }

    // The saved session that rested opens with its login, for the token allowed to use it; the revoked token is
    // still refused.
    let resume = json!({"url": PAGE, "resume": rested});
    let (status, later) = agent.rest(Method::POST, "/api/agent/sessions", Some(resume)).await;
    assert_eq!(status, StatusCode::OK, "{later}");
    let later = later["id"].as_str().expect("an id").to_owned();
    admin.adopt(&later);
    shows(&admin, &later, "login: rested").await;
    assert_eq!(revoked.rest(Method::GET, "/api/agent/tools", None).await.0, StatusCode::UNAUTHORIZED);

    // The egress is there with its place, and its check passes: what it leaves through is still read.
    let egresses = admin.get("/api/egresses").await.expect("the egresses");
    let kept_egress = egresses["egresses"].as_array().expect("a list").iter().find(|e| e["id"] == json!(egress));
    let place = kept_egress.map(|e| (e["timezone"].clone(), e["languages"].clone()));
    assert_eq!(place, Some((json!("Europe/Berlin"), json!(["de-DE", "de"]))), "{egresses}");
    let check = admin.egress_check(&egress).await;
    assert_eq!(check["ok"], true, "the egress no longer passes its check: {check}");

    // The replay and the timeline kept before the update open behind the password.
    let runs = admin.get("/api/runs").await.expect("the runs");
    assert!(runs["runs"].as_array().unwrap().iter().any(|r| r["id"] == json!(recorded)), "it is listed: {runs}");
    assert_eq!(runs["locked"], 0, "the password opened them: {runs}");
    let timeline = admin.get(&format!("/api/sessions/{recorded}/timeline?after=0")).await.expect("its timeline");
    assert!(timeline["events"].as_array().is_some_and(|e| !e.is_empty()), "its events: {timeline}");
    let index = admin.get(&replay).await.expect("its replay");
    let n = index["segments"][0]["n"].as_u64().expect("a segment");
    let raw = admin.http.get(format!("{}{replay}/{n}", admin.origin)).header("cookie", &admin.cookie).send().await;
    let segment = common::replay_records(&raw.expect("the segment").bytes().await.expect("its bytes"));
    let whole = segment.iter().filter(|(_, kind, _)| *kind == 0).any(|(_, _, bytes)| {
        let message = Message::decode(bytes, display_protocol::Direction::ServerToClient).expect("a message");
        matches!(message, Message::Video { keyframe: true, .. })
    });
    assert!(whole, "the segment opens whole");

    admin.clear().await;
    admin.call(Method::DELETE, &format!("/api/chat-accounts/{account}"), None).await.expect("the chat account goes");
    for kind in ["profiles", "tokens", "egresses"] {
        admin.remove_named(kind, |n| n.starts_with(NAME)).await;
    }
}
