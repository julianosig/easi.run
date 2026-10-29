//! Control, quiescence, epochs, and redaction (docs/design.md §3.6), through the running admin application with a
//! scripted agent (no model).

mod common;

use std::time::{Duration, Instant};

use common::{Admin, FIELD, TYPING as FIXTURE, Viewer, counts};
use display_protocol::{LeaseHolder, Message};
use serde_json::{Value, json};

async fn window(admin: &Admin, session: &str) -> (u64, u64) {
    admin.wait_windows(session, "the browser window", common::first_window).await
}

/// What the agent reads of the page now: its accessibility tree and the text field's value.
async fn observe(admin: &Admin, session: &str, (pid, window): (u64, u64)) -> String {
    let tree =
        admin.agent(session, "get_window_state", json!({"pid": pid, "window_id": window})).await.expect("a tree");
    let script = format!(
        "const app = await cua.getApp({{pid: {pid}, windowId: {window}}}); \
         const fields = await app.query({{role: 'entry', name: 'Type here'}}); return fields.map(f => f.value);"
    );
    let value = admin.agent(session, "run_script", json!({"script": script})).await.expect("the field's value");
    format!("{tree} {value}")
}

async fn sleep_script(admin: &Admin, session: &str, ms: u64) -> Result<Value, common::Refused> {
    let script = format!("await cua.sleep({ms}); return 'slept';");
    admin.agent(session, "run_script", json!({"script": script, "timeout_ms": 120000})).await
}

/// A `ms` sleep script on the agent, from a login of its own, in a task of its own.
async fn sleeping(session: &str, ms: u64) -> tokio::task::JoinHandle<Result<Value, common::Refused>> {
    let (admin, session) = (Admin::login().await, session.to_owned());
    tokio::spawn(async move { sleep_script(&admin, &session, ms).await })
}

/// A session of the typing fixture whose agent has just listed its window, and a viewer opened since, which watches:
/// the agent is using the session.
async fn setup() -> (Admin, String, Viewer, (u64, u64)) {
    let admin = Admin::login().await;
    let session = admin.create(FIXTURE).await;
    let target = window(&admin, &session).await;
    let person = admin.watch(&session, 0).await;
    person.wait_lease(LeaseHolder::Agent).await;
    (admin, session, person, target)
}

/// The middle of the typing fixture's square (200 pixels, 40 from the page's top right), which every key press flips
/// between black and white, and a point of the white page 20 pixels left of the square.
const FLAG: (u16, u16) = (1280 - 40 - 100, common::PAGE_TOP + 40 + 100);
const BESIDE: (u16, u16) = (1280 - 40 - 200 - 20, FLAG.1);

/// The screen's pixel at `(x, y)`, as the newest lossless tile over it shows it.
fn pixel(viewer: &Viewer, (x, y): (u16, u16)) -> Option<[u8; 3]> {
    let seen = viewer.seen.lock().unwrap();
    seen.log.iter().rev().find_map(|(_, message)| {
        let Message::Tiles { rects, raw_len, data, .. } = message else { return None };
        let at = rects.iter().position(|r| r.x <= x && x < r.x + r.w && r.y <= y && y < r.y + r.h)?;
        let rect = rects[at];
        let rgb = &common::tiles(rects, *raw_len, data)[at];
        let i = 3 * (usize::from(y - rect.y) * usize::from(rect.w) + usize::from(x - rect.x));
        Some([rgb[i], rgb[i + 1], rgb[i + 2]])
    })
}

/// Whether the screen at `(x, y)` is white (`Some(true)`) or black, as the newest lossless tile over it shows it.
fn shade(viewer: &Viewer, at: (u16, u16)) -> Option<bool> {
    let rgb = pixel(viewer, at)?;
    if rgb.iter().all(|&c| c > 240) {
        Some(true)
    } else if rgb.iter().all(|&c| c < 16) {
        Some(false)
    } else {
        None
    }
}

/// Waits until the square shows `white` on the white page beside it: the fixture drew it (the screen the compositor
/// shows before any window is black all over, and a page loading is white all over).
async fn wait_flag(viewer: &Viewer, white: bool) {
    let shown = common::until!(600, Duration::from_millis(100), {
        (shade(viewer, BESIDE) == Some(true) && shade(viewer, FLAG) == Some(white)).then_some(())
    });
    assert!(shown.is_some(), "the square never turned {}", if white { "white" } else { "black" });
}

#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_fresh_session_opens_holding_control_and_takes_input_at_once() {
    let admin = Admin::login().await;
    let session = admin.create(FIXTURE).await;
    // The page opens the session nobody holds and whose agent has done nothing: its viewer is given control at once.
    let person = admin.watch(&session, 0).await;
    let epoch = person.wait_lease(LeaseHolder::You).await;
    let told = person.seen.lock().unwrap().leases.clone();
    assert!(told.iter().all(|l| l.1 == LeaseHolder::You), "the viewer was told another holder first: {told:?}");
    // Its keys reach the page without a takeover: once the page has drawn its black square, each key press flips it.
    wait_flag(&person, false).await;
    person.type_keys(epoch, "a").await;
    wait_flag(&person, true).await;
    person.type_keys(epoch, "b").await;
    wait_flag(&person, false).await;
    // Holding control still blocks the agent, until the person gives it back.
    let blocked = admin.agent(&session, "list_windows", json!({})).await.expect_err("the agent is blocked");
    assert_eq!(blocked.0, 409);
    person.type_keys(epoch, "secret").await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    admin.hand_back(&session, &person).await;
    // As after any takeover: what the person typed reaches the agent redacted, and screenshots never do.
    let received = counts(&admin, &session).await;
    assert_eq!(common::count(&received, "keys"), Some(8), "every key reached the page: {received}");
    let seen = observe(&admin, &session, window(&admin, &session).await).await;
    assert!(!seen.contains("secret") && seen.contains("[redacted]"), "the person's typing reached the agent: {seen}");
    let shot = admin.agent(&session, "get_desktop_state", json!({})).await.expect("the desktop's state");
    let content = shot["content"].as_array().filter(|_| shot["isError"] != true).expect("a result's content");
    assert!(content.iter().all(|i| i["type"] != "image"), "a screenshot reached the agent: {shot}");
    person.close().await;
    admin.stop(&session).await;
}

#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_session_the_agent_has_just_used_opens_watching() {
    let admin = Admin::login().await;
    let session = admin.create(FIXTURE).await;
    window(&admin, &session).await;
    // An agent call has just ended: the viewer opens watching, and the agent goes on acting.
    let person = admin.watch(&session, 0).await;
    person.wait_lease(LeaseHolder::Agent).await;
    let told = person.seen.lock().unwrap().leases.clone();
    assert!(told.iter().all(|l| l.1 == LeaseHolder::Agent), "the viewer was given control: {told:?}");
    admin.agent(&session, "list_windows", json!({})).await.expect("the agent acts");
    // One take gives the person control.
    admin.take(&session, &person).await;
    person.close().await;
    admin.stop(&session).await;
}

#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn takeover_waits_for_the_in_flight_call_and_blocks_the_agent() {
    let (admin, session, person, target) = setup().await;
    let watcher = admin.watch(&session, 0).await;
    watcher.wait_lease(LeaseHolder::Agent).await;

    // A two-second agent command is in flight when the person asks for control.
    let started = Instant::now();
    let in_flight = sleeping(&session, 2000).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let granted = admin.take_control(&session, &person).await.expect("control is granted");
    let waited = started.elapsed();
    let answered = in_flight.await.unwrap().expect("the in-flight call completed");
    assert!(!answered["structuredContent"].is_null(), "the in-flight call answered: {answered}");
    assert!(waited >= Duration::from_millis(1900), "control was granted before the in-flight call ended ({waited:?})");
    let epoch = person.wait_lease(LeaseHolder::You).await;
    assert_eq!(granted["lease_epoch"].as_u64(), Some(u64::from(epoch)));
    watcher.wait_lease(LeaseHolder::Other).await;

    // No agent command runs while the person holds control.
    let blocked = admin.agent(&session, "list_windows", json!({})).await.expect_err("the agent is blocked");
    assert_eq!(blocked.0, 409);

    // Only the holder's input at the current epoch reaches the page: the watcher's and the holder's own at a stale epoch
    // do not. The page counts what it receives (the agent would read leaked text redacted, so text cannot tell).
    person.click(epoch, FIELD.0, FIELD.1).await;
    person.type_keys(epoch, "holder").await;
    watcher.click(epoch, FIELD.0, FIELD.1).await;
    watcher.type_keys(epoch, "watcher").await;
    person.click(epoch.wrapping_sub(2), FIELD.0, FIELD.1).await;
    person.type_keys(epoch.wrapping_sub(2), "stale").await;
    tokio::time::sleep(Duration::from_millis(500)).await;

    admin.give_back(&session, &person).await.expect("control is given back");
    let back = person.wait_lease(LeaseHolder::Agent).await;
    assert!(back > epoch, "give back moves to a newer epoch");
    let received = counts(&admin, &session).await;
    assert_eq!(
        (common::count(&received, "clicks"), common::count(&received, "keys")),
        (Some(1), Some(6)),
        "input without the lease reached the page: {received}"
    );
    let seen = observe(&admin, &session, target).await;
    // "holder" is six characters typed by the person: the agent sees it redacted, never in the clear.
    assert!(!seen.contains("holder"), "the person's typing reached the agent: {seen}");
    assert!(seen.contains("[redacted]"), "the field shows the redaction mark: {seen}");

    watcher.close().await;
    person.close().await;
    admin.stop(&session).await;
}

#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_command_that_never_answers_ends_with_the_agent_killed_and_proven_stopped() {
    let (admin, session, person, _) = setup().await;
    let stuck = sleeping(&session, 100_000).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let started = Instant::now();
    admin.take_control(&session, &person).await.expect("control is granted once the agent is proven stopped");
    let waited = started.elapsed();
    assert!(waited >= Duration::from_secs(5) && waited < Duration::from_secs(30), "quiescence bound: {waited:?}");
    assert!(stuck.await.unwrap().is_err(), "the stuck command did not complete; it ended with the agent");
    person.wait_lease(LeaseHolder::You).await;
    // Give back restarts a fresh agent that acts again.
    admin.give_back(&session, &person).await.expect("give back restarts the agent");
    person.wait_lease(LeaseHolder::Agent).await;
    window(&admin, &session).await;
    person.close().await;
    admin.stop(&session).await;
}

#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn the_lease_is_revoked_when_its_holder_disconnects() {
    let (admin, session, person, _) = setup().await;
    let watcher = admin.watch(&session, 0).await;
    watcher.wait_lease(LeaseHolder::Agent).await;
    admin.take_control(&session, &person).await.expect("control");
    let held = watcher.wait_lease(LeaseHolder::Other).await;
    person.close().await;
    let after = watcher.wait_lease(LeaseHolder::Agent).await;
    assert!(after > held);
    admin.agent(&session, "list_windows", json!({})).await.expect("the agent acts again");
    watcher.close().await;
    admin.stop(&session).await;
}

#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn give_back_follows_a_64_kib_paste_and_a_full_redaction_set_hides_what_the_agent_reads() {
    let (admin, session, person, _) = setup().await;
    // Printable text of no repeated stretch, as large as a paste may be.
    let paste = |mut seed: u64| -> String {
        (0..64 * 1024)
            .map(|_| {
                seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
                char::from(b'!' + ((seed >> 33) % 94) as u8)
            })
            .collect()
    };
    let epoch = admin.take(&session, &person).await;
    person.send(Message::Clipboard { lease_epoch: epoch, text: paste(0) }).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    admin.give_back(&session, &person).await.expect("give back after a 64 KiB paste");
    person.wait_lease(LeaseHolder::Agent).await;
    let read = admin.agent(&session, "list_windows", json!({})).await.expect("a result");
    assert_ne!(read["isError"], true, "{read}");
    // More than the set holds: give back still goes on, and nothing the agent reads is passed on.
    let epoch = admin.take(&session, &person).await;
    for seed in 1..6 {
        person.send(Message::Clipboard { lease_epoch: epoch, text: paste(seed) }).await;
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    admin.give_back(&session, &person).await.expect("give back with a full set");
    person.wait_lease(LeaseHolder::Agent).await;
    let read = admin.agent(&session, "list_windows", json!({})).await.expect("a result");
    assert_eq!(read["isError"], true, "{read}");
    assert!(read.to_string().contains("[redacted]"), "{read}");
    person.close().await;
    admin.stop(&session).await;
}

#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_modified_watcher_injects_nothing_and_a_revoked_lease_is_refused() {
    let (admin, session, person, _) = setup().await;
    loaded(&admin, &session).await;
    let before = counts(&admin, &session).await;
    assert!(before.contains("moves 0 clicks 0 keys 0 pastes 0 wheels 0"), "{before}");
    // A watcher with its client-side guard removed sends every kind of input at every plausible epoch.
    let watcher = admin.watch(&session, 0).await;
    let epoch = watcher.wait_lease(LeaseHolder::Agent).await;
    for e in [0, epoch, epoch + 1, epoch + 2, u32::MAX] {
        watcher.click(e, FIELD.0, FIELD.1).await;
        watcher.type_keys(e, "abc").await;
        watcher.send(Message::Wheel { lease_epoch: e, x: 600, y: 400, dx: 0, dy: 120 }).await;
        watcher.send(Message::Clipboard { lease_epoch: e, text: "pasted".into() }).await;
    }
    tokio::time::sleep(Duration::from_millis(800)).await;
    let after = counts(&admin, &session).await;
    assert!(after.contains("moves 0 clicks 0 keys 0 pastes 0 wheels 0"), "a watcher's input reached the page: {after}");
    // A message the protocol does not know (a "resize") closes that viewer's stream; the screen keeps its size.
    let configs = watcher.seen.lock().unwrap().configs;
    watcher.send_raw(vec![0x7e, 0, 5, 0, 3]).await;
    watcher.wait_for("the stream's end after an unknown message", |s| s.closed.then_some(())).await;
    assert_eq!(watcher.seen.lock().unwrap().configs, configs, "the screen was reconfigured");

    // The holder's input reaches the page; after give back, the same epoch is refused.
    let held = admin.take(&session, &person).await;
    person.click(held, FIELD.0, FIELD.1).await;
    person.type_keys(held, "ok").await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    admin.hand_back(&session, &person).await;
    let granted = counts(&admin, &session).await;
    assert!(
        granted.contains("clicks 1") && granted.contains("keys 2"),
        "the holder's input reached the page: {granted}"
    );
    person.click(held, FIELD.0, FIELD.1).await;
    person.type_keys(held, "late").await;
    tokio::time::sleep(Duration::from_millis(800)).await;
    let revoked = counts(&admin, &session).await;
    assert!(
        revoked.contains("clicks 1") && revoked.contains("keys 2"),
        "input after revocation reached the page: {revoked}"
    );
    person.close().await;
    admin.stop(&session).await;
}

/// Waits until the page has loaded and drawn: its counters reach the accessibility tree.
async fn loaded(admin: &Admin, session: &str) {
    let shown = common::until!(60, Duration::from_millis(500), {
        counts(admin, session).await.contains("pastes 0").then_some(())
    });
    assert!(shown.is_some(), "the page never showed its counters");
}

/// Waits for a cursor message with `shape` after `from` messages of the log.
async fn cursor_shape(viewer: &Viewer, from: usize, shape: u8) {
    let shown = common::until!(60, Duration::from_millis(100), {
        let log = viewer.log();
        log.iter().skip(from).any(|(_, m)| matches!(m, Message::Cursor { shape: s, .. } if *s == shape)).then_some(())
    });
    if shown.is_some() {
        return;
    }
    let seen: Vec<(u8, usize)> = viewer
        .log()
        .iter()
        .filter_map(|(_, m)| match m {
            Message::Cursor { shape, image, .. } => Some((*shape, image.len())),
            _ => None,
        })
        .collect();
    panic!("no cursor with shape {shape} arrived; cursors seen (shape, image bytes): {seen:?}");
}

#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn the_pointer_shape_follows_the_page_under_the_holders_pointer() {
    let (admin, session, person, _) = setup().await;
    loaded(&admin, &session).await;
    let watcher = admin.watch(&session, 0).await;
    let held = admin.take(&session, &person).await;
    let from = watcher.log().len();
    let pointer = |x, y| person.pointer(held, display_protocol::PointerKind::Move, x, y);
    // Over the text field Chromium shows the text cursor (shape 9); over the empty page, the default arrow (1).
    pointer(FIELD.0, FIELD.1).await;
    cursor_shape(&watcher, from, 9).await;
    let from = watcher.log().len();
    pointer(640, 600).await;
    cursor_shape(&watcher, from, 1).await;
    watcher.close().await;
    person.close().await;
    admin.stop(&session).await;
}

#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn the_holders_clipboard_pastes_once_and_the_agent_reads_it_redacted() {
    let (admin, session, person, target) = setup().await;
    loaded(&admin, &session).await;
    let held = admin.take(&session, &person).await;
    person.click(held, FIELD.0, FIELD.1).await;
    person.send(Message::Clipboard { lease_epoch: held, text: "Paste me 1".into() }).await;
    tokio::time::sleep(Duration::from_millis(800)).await;
    admin.hand_back(&session, &person).await;
    let after = counts(&admin, &session).await;
    assert!(after.contains("pastes 1"), "the page saw exactly one paste: {after}");
    let seen = observe(&admin, &session, target).await;
    assert!(!seen.contains("Paste me 1"), "the agent read the pasted text: {seen}");
    assert!(seen.contains("[redacted]") && !seen.contains("[redacted][redacted]"), "one redacted paste: {seen}");
    // Giving control back cleared the clipboard: the agent's own Ctrl+V reaches the page and pastes nothing.
    let before = common::count(&after, "keys").expect("a key count");
    common::agent_ctrl(&admin, &session, target, "v", before).await;
    let seen = observe(&admin, &session, target).await;
    assert!(!seen.contains("[redacted][redacted]"), "the paste was replayed after give back: {seen}");
    person.close().await;
    admin.stop(&session).await;
}

#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_long_paste_arrives_whole_and_what_the_page_copies_reaches_the_holder_alone() {
    let (admin, session, person, _) = setup().await;
    loaded(&admin, &session).await;
    let watcher = admin.watch(&session, 0).await;
    let held = admin.take(&session, &person).await;
    person.click(held, FIELD.0, FIELD.1).await;
    // Far more than a pipe holds at once: the browser reads it in pieces, and all of it arrives.
    let text: String = (0..50_000).map(|i| char::from(b'a' + (i % 26) as u8)).collect();
    person.send(Message::Clipboard { lease_epoch: held, text: text.clone() }).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    // The person selects the field and copies it: the text comes back to the holder, at its lease epoch, whole (so the
    // paste arrived whole too).
    use display_protocol::keys::{C, LEFTCTRL};
    const A: u16 = 30;
    for code in [A, C] {
        person.chord(held, &[LEFTCTRL, code]).await;
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    person.wait_for("the page's copy", |s| (!s.copied.is_empty()).then_some(())).await;
    let copied = person.seen.lock().unwrap().copied.clone();
    assert_eq!(copied.len(), 1, "one copy reached the holder");
    assert_eq!(copied[0].0, held);
    assert!(copied[0].1 == text, "the copied text is the field's ({} bytes)", copied[0].1.len());
    assert!(watcher.seen.lock().unwrap().copied.is_empty(), "a watcher received what the page copied");
    watcher.close().await;
    person.close().await;
    admin.stop(&session).await;
}

/// The document row shown at screen row `y` of the wheel fixture, from the newest lossless tile that covers its strip.
fn document_row(viewer: &Viewer, y: u16) -> Option<u32> {
    let [r, g, b] = pixel(viewer, (8, y))?;
    (b == 128).then(|| u32::from(r) << 8 | u32::from(g))
}

/// How far the page has scrolled once it settles: the row at screen row 400, unchanged for a second.
async fn settled_scroll(viewer: &Viewer) -> i64 {
    let (mut last, mut since) = (None, Instant::now());
    for _ in 0..200 {
        let now = document_row(viewer, 400);
        if now != last {
            (last, since) = (now, Instant::now());
        } else if now.is_some() && since.elapsed() >= Duration::from_secs(1) {
            return i64::from(now.unwrap()) - i64::from(400 - common::PAGE_TOP);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the page never settled (last row {last:?})");
}

#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_wheel_scrolls_the_page_by_its_pixels() {
    let admin = Admin::login().await;
    let session = admin.create("https://fixture.bench/wheel.html").await;
    // A fresh session: the viewer opens holding control.
    let person = admin.watch(&session, 0).await;
    let epoch = person.wait_lease(LeaseHolder::You).await;
    assert_eq!(settled_scroll(&person).await, 0, "the page starts at the top");
    // One mouse-wheel notch as Chromium reports it, then a trackpad's run of small deltas, then a step back up.
    let mut expected = 0;
    for (steps, dy) in [(1, 100), (40, 5), (3, -37)] {
        for _ in 0..steps {
            person.send(Message::Wheel { lease_epoch: epoch, x: 640, y: 400, dx: 0, dy }).await;
            tokio::time::sleep(Duration::from_millis(16)).await;
        }
        expected += steps * i64::from(dy);
        assert_eq!(settled_scroll(&person).await, expected, "after {steps} wheel events of {dy} px");
    }
    admin.stop(&session).await;
}

#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_new_window_shows_on_top_and_closing_it_returns_to_the_page() {
    use display_protocol::keys::{N, W};
    let admin = Admin::login().await;
    let session = admin.create(FIXTURE).await;
    // The page has loaded and the window is active before the person types.
    observe(&admin, &session, window(&admin, &session).await).await;
    let person = admin.watch(&session, 0).await;
    let held = admin.take(&session, &person).await;
    ctrl(&person, held, N).await;
    admin.hand_back(&session, &person).await;
    // The key was delivered while the person held control; Chrome may map the window only after give back.
    admin.wait_windows(&session, "a second window, which Ctrl+N opens,", |w| (w.len() == 2).then_some(())).await;
    // The agent's input by PID names no window, so with two shown it is refused rather than sent to the wrong one.
    let (pid, id) = window(&admin, &session).await;
    let typed = admin.agent(&session, "type_text", json!({"pid": pid, "window_id": id, "text": "x"})).await;
    assert!(typed.is_err() || typed.as_ref().unwrap().to_string().contains("ambiguous"), "{typed:?}");
    let held = admin.take(&session, &person).await;
    // The new window is the one on top and focused: closing it with the keyboard works, and the page is back.
    ctrl(&person, held, W).await;
    // A hand reaches the field before it clicks: the window just regained focus.
    person.pointer(held, display_protocol::PointerKind::Move, FIELD.0, FIELD.1).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    person.click(held, FIELD.0, FIELD.1).await;
    person.type_keys(held, "abc").await;
    // The input travels the stream before control is given back, which would drop what is still on its way.
    tokio::time::sleep(Duration::from_secs(1)).await;
    admin.hand_back(&session, &person).await;
    admin
        .wait_windows(&session, "the page's window alone, once Ctrl+W closes the new one,", |w| {
            (w.len() == 1).then_some(())
        })
        .await;
    let seen = observe(&admin, &session, window(&admin, &session).await).await;
    assert!(seen.contains("abc"), "the page's field took the keys again: {seen}");
    person.close().await;
    admin.stop(&session).await;
}

/// Presses Ctrl+`key` as a hand would, a few tens of milliseconds apart, and gives the browser two seconds.
async fn ctrl(person: &Viewer, held: u32, key: u16) {
    use display_protocol::keys::LEFTCTRL;
    for (code, down) in [(LEFTCTRL, true), (key, true), (key, false), (LEFTCTRL, false)] {
        person.key(held, code, down).await;
        tokio::time::sleep(Duration::from_millis(60)).await;
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
}

/// The title of the window the agent lists first.
async fn title(admin: &Admin, session: &str) -> String {
    let result = admin.agent(session, "list_windows", json!({})).await.expect("a list");
    result["structuredContent"]["windows"][0]["title"].as_str().unwrap_or_default().to_owned()
}

/// The viewport size a title of the size fixture shows (`size WxH - Google Chrome`), once its script has run.
fn size_of(title: &str) -> Option<&str> {
    let size = title.strip_prefix("size ")?.split_whitespace().next()?;
    let (w, h) = size.split_once('x')?;
    (w.parse::<u16>().is_ok() && h.parse::<u16>().is_ok()).then_some(size)
}

/// The first window's title once its size satisfies `wanted`, or the last title seen after ten seconds: the page reports
/// each size it is given as the browser draws at it, so a resize may pass through others (slower on a loaded host).
async fn title_once(admin: &Admin, session: &str, wanted: impl Fn(&str) -> bool) -> String {
    let mut last = String::new();
    common::until!(40, Duration::from_millis(250), {
        last = title(admin, session).await;
        size_of(&last).is_some_and(&wanted).then_some(())
    });
    last
}

#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_page_goes_fullscreen_and_back() {
    use display_protocol::keys::F11;
    let admin = Admin::login().await;
    let session = admin.create("https://fixture.bench/size.html").await;
    // The page's window alone: Chrome's notices of its first seconds are windows of their own, which hold the keyboard
    // (a key pressed meanwhile reaches them, not the page).
    let maximized = admin
        .wait_windows(&session, "the size page's window alone", |windows| match windows {
            [w] => w["title"].as_str().filter(|t| size_of(t).is_some()).map(str::to_owned),
            _ => None,
        })
        .await;
    assert!(maximized.starts_with("size 1280x"), "{maximized}");
    let maximized = size_of(&maximized).unwrap().to_owned();
    let person = admin.watch(&session, 0).await;
    let press = |code, wanted: String| {
        let person = &person;
        let admin = &admin;
        let session = &session;
        async move {
            let held = admin.take(session, person).await;
            let from = person.log().len();
            person.key(held, code, true).await;
            tokio::time::sleep(Duration::from_millis(60)).await;
            person.key(held, code, false).await;
            // Given back only once the browser has redrawn for the key (the page itself is still): a lease change
            // drops the input still on its way.
            person
                .wait_for("the browser's redraw for the key", |s| {
                    s.log[from..].iter().any(|(_, m)| matches!(m, Message::Tiles { .. })).then_some(())
                })
                .await;
            admin.hand_back(session, person).await;
            title_once(admin, session, |size| size == wanted).await
        }
    };
    let full = press(F11, "1280x800".to_owned()).await;
    assert!(full.starts_with("size 1280x800"), "fullscreen covers the screen: {full}");
    let back = press(F11, maximized.clone()).await;
    assert_eq!(size_of(&back), Some(maximized.as_str()), "leaving fullscreen returns to the maximized window: {back}");
    person.close().await;
    admin.stop(&session).await;
}

/// A site's permission prompt (Chrome draws it in a subsurface above the page) shows, and the person answers it: here,
/// a position request, which Chrome's own provider answers with where the browser's traffic leaves from.
#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_permission_prompt_shows_and_the_person_answers_it() {
    let admin = Admin::login().await;
    let session = admin.create("https://fixture.bench/geolocation.html").await;
    let (pid, id) = window(&admin, &session).await;
    // The agent asks for the position; the prompt's buttons are read, not clicked, so the person's click decides.
    let script = format!(
        "const app = await cua.getApp({{pid: {pid}, windowId: {id}}}); await app.click({{role: 'button', name: 'Locate'}}); \
         for (let i = 0; i < 20; i++) {{ await cua.sleep(500); \
           const b = (await app.query({{role: 'button'}})).find(x => x.label === 'Permitir desta vez'); if (b) return b.frame; }} \
         return null;"
    );
    let found = admin.agent(&session, "run_script", json!({"script": script})).await.expect("the prompt's button");
    let frame = &found["structuredContent"]["value"];
    let (Some(x), Some(y), Some(w), Some(h)) =
        (frame["x"].as_u64(), frame["y"].as_u64(), frame["w"].as_u64(), frame["h"].as_u64())
    else {
        panic!("no prompt was shown: {found}");
    };
    let person = admin.watch(&session, 0).await;
    let held = admin.take(&session, &person).await;
    // The window sits at the output's origin, so the button's place in the window is its place on the screen.
    person.click(held, (x + w / 2) as u16, (y + h / 2) as u16).await;
    let script = format!(
        "for (let i = 0; i < 40; i++) {{ await cua.sleep(500); \
           const t = (await cua.listWindows({{pid: {pid}}})).map(w => w.title).join(' | '); \
           if (t.includes('position ') || t.includes('error ')) return t; }} return 'no answer';"
    );
    tokio::time::sleep(Duration::from_secs(1)).await;
    admin.hand_back(&session, &person).await;
    let answer = admin.agent(&session, "run_script", json!({"script": script, "timeout_ms": 60000})).await.unwrap();
    let title = answer["structuredContent"]["value"].as_str().unwrap_or_default().to_owned();
    // "position LAT LON accuracy METRES age MS": a real position, measured just now.
    let words: Vec<&str> = title.split_whitespace().collect();
    assert_eq!(words.first(), Some(&"position"), "{title}");
    let accuracy: f64 = words[4].parse().unwrap_or(0.0);
    let age: f64 = words[6].trim_end_matches(':').parse().unwrap_or(f64::MAX);
    assert!(accuracy > 0.0 && age < 10_000.0, "{title}");
    person.close().await;
    admin.stop(&session).await;
}
