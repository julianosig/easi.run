//! A session's features: what one that is off would carry never reaches a viewer or the page, whatever the client
//! sends, and the session lists them as they take effect. Runs only with `DISPLAY_E2E=1` against the bench profile.

mod common;

use std::time::Duration;

use common::{Admin, Viewer, count, counts, enabled, page_text};
use display_protocol::{LeaseHolder, Message};
use serde_json::{Value, json};

const SAVE: &str = "https://fixture.bench/save.html";
const UPLOAD: &str = "https://fixture.bench/upload.html";
const TYPING: &str = "https://fixture.bench/typing.html";

/// A session on `url` with `features`, its listed features, and a viewer holding control once the page titled
/// `title` loaded.
async fn holding(admin: &Admin, url: &str, title: &str, features: Value) -> (String, Value, Viewer, u32) {
    let session = admin.create_with(json!({"url": url, "features": features})).await;
    let listed = admin.get("/api/sessions").await.expect("the sessions");
    let entry = listed["sessions"].as_array().expect("a list").iter().find(|s| s["id"] == session.as_str());
    let features = entry.expect("the new session listed")["features"].clone();
    let person = admin.watch(&session, 0).await;
    admin.wait_title(&session, title).await;
    admin.take_control(&session, &person).await.expect("control");
    let held = person.wait_lease(LeaseHolder::You).await;
    (session, features, person, held)
}

#[tokio::test]
async fn with_downloads_off_a_file_the_page_saves_is_never_offered_and_recording_is_off_too() {
    if !enabled() {
        return;
    }
    let admin = Admin::login().await;
    let (session, features, person, mut held) = holding(&admin, SAVE, "Save", json!({"downloads": false})).await;
    assert_eq!((&features["downloads"], &features["recording"]), (&json!(false), &json!(false)), "{features}");
    assert_eq!(features["clipboard"], json!(true), "{features}");
    // The click saves through the page's save dialog, and the page notes how that ended (a click before the page is
    // live does nothing; the agent reads the page once control is back, and the person then holds it again, so a
    // download still waiting would be offered to them).
    let mut text = String::new();
    for _ in 0..3 {
        person.click(held, 640, 400).await;
        tokio::time::sleep(Duration::from_secs(3)).await;
        admin.give_back(&session, &person).await.expect("give back");
        person.wait_lease(LeaseHolder::Agent).await;
        text = page_text(&admin, &session).await;
        admin.take_control(&session, &person).await.expect("control");
        held = person.wait_lease(LeaseHolder::You).await;
        if text.contains("Saved.") || text.contains("save refused") {
            break;
        }
    }
    assert!(text.contains("Saved.") || text.contains("save refused"), "the page never tried its save: {text}");
    tokio::time::sleep(Duration::from_secs(2)).await;
    {
        let seen = person.seen.lock().unwrap();
        assert!(seen.offers.is_empty() && seen.links.is_empty(), "a download was offered: {:?}", seen.offers);
    }
    person.close().await;
    admin.stop(&session).await;
}

#[tokio::test]
async fn with_keyboard_and_pointer_off_the_holders_input_never_reaches_the_page() {
    if !enabled() {
        return;
    }
    let admin = Admin::login().await;
    let (session, _, person, held) = holding(&admin, TYPING, "Typing", json!({"keyboard_pointer": false})).await;
    // The holder clicks, types and scrolls over the page, which counts every input event it receives.
    person.click(held, 400, 62 + common::PAGE_TOP).await;
    person.type_keys(held, "abc").await;
    person.send(Message::Wheel { lease_epoch: held, x: 640, y: 400, dx: 0, dy: 120 }).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    admin.give_back(&session, &person).await.expect("give back");
    person.wait_lease(LeaseHolder::Agent).await;
    let after = counts(&admin, &session).await;
    for counter in ["clicks", "keys", "wheels"] {
        assert_eq!(count(&after, counter), Some(0), "the holder's input reached the page: {after}");
    }
    // The page was live and counting: the agent's own keys reach it.
    let (pid, window) = admin.wait_windows(&session, "the page's window", common::first_window).await;
    let pressed =
        admin.agent(&session, "hotkey", json!({"pid": pid, "window_id": window, "keys": ["ctrl", "a"]})).await;
    assert!(pressed.as_ref().is_ok_and(|r| r["isError"] != json!(true)), "the agent's Ctrl+A: {pressed:?}");
    let mut counted = String::new();
    for _ in 0..20 {
        counted = counts(&admin, &session).await;
        if count(&counted, "keys").is_some_and(|n| n > 0) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert!(count(&counted, "keys").is_some_and(|n| n > 0), "the agent's keys did not reach the page: {counted}");
    person.close().await;
    admin.stop(&session).await;
}

#[tokio::test]
async fn with_uploads_off_the_pages_file_chooser_never_asks_the_holder_and_gets_nothing() {
    if !enabled() {
        return;
    }
    let admin = Admin::login().await;
    let (session, _, person, mut held) = holding(&admin, UPLOAD, "Upload", json!({"uploads": false})).await;
    // The click opens the page's file chooser: the person is never asked, and the chooser is answered empty, which
    // the page notes (a click before the page is live opens nothing; the agent reads the page once control is back).
    let mut text = String::new();
    for _ in 0..3 {
        person.click(held, 640, 400).await;
        tokio::time::sleep(Duration::from_secs(3)).await;
        admin.give_back(&session, &person).await.expect("give back");
        person.wait_lease(LeaseHolder::Agent).await;
        text = page_text(&admin, &session).await;
        admin.take_control(&session, &person).await.expect("control");
        held = person.wait_lease(LeaseHolder::You).await;
        if text.contains("chooser closed") {
            break;
        }
    }
    assert!(text.contains("chooser closed"), "the page's chooser never opened and closed empty: {text}");
    assert!(person.seen.lock().unwrap().requests.is_empty(), "the holder was asked for files");
    // An upload the client sends anyway is never acknowledged or stored, and dropping it on the page goes nowhere.
    let bytes = b"an upload the session refuses".to_vec();
    let size = bytes.len() as u64;
    person.send(Message::UploadStart { lease_epoch: held, upload: 1, size, name: "refused.txt".into() }).await;
    person.send(Message::UploadData { lease_epoch: held, upload: 1, offset: 0, data: bytes }).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    {
        let seen = person.seen.lock().unwrap();
        assert_eq!((&seen.acks[..], &seen.uploaded[..]), (&[][..], &[][..]), "an upload was acknowledged or stored");
    }
    person.send(Message::FileDrop { lease_epoch: held, x: 640, y: 400, uploads: vec![1] }).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    admin.give_back(&session, &person).await.expect("give back");
    person.wait_lease(LeaseHolder::Agent).await;
    let text = page_text(&admin, &session).await;
    assert!(!text.contains("chosen ") && !text.contains("dropped "), "a file reached the page: {text}");
    person.close().await;
    admin.stop(&session).await;
}
