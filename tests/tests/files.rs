//! Files between the person and the browser (docs/design.md §3.6): a download reaches the person holding control,
//! byte-exact, through the single-use ticket of its native download, and nobody else; one far larger than the browser's
//! memory streams at the person's pace; a cancelled one, one whose lease moves, or one whose connection drops and that
//! Chromium starts again ends as a failure, never mixed; a file the person gives the page's file chooser, or drops on
//! the page, reaches the page byte-exact.

mod common;

// The fixture host's download bytes, read from its source.
#[path = "../../bench/src/pattern.rs"]
mod pattern;

use std::time::{Duration, Instant};

use common::{ARTICLE, Admin, INSTANCE, SAVE, UPLOAD, Viewer, page_text};
use display_protocol::{FILE_TICKET, LeaseHolder, Message, UPLOAD_CHUNK, UPLOAD_WINDOW};
use pattern::big_byte;

const DOWNLOAD: &str = "https://fixture.bench/download.html";
const DROP_ZONE: &str = "https://fixture.bench/drop.html";

/// The fixture's file: byte i is (i * 7 + i >> 8) & 255.
fn expected() -> Vec<u8> {
    (0..300_000usize).map(|i| ((i * 7 + (i >> 8)) & 255) as u8).collect()
}

/// The offer of the first download this viewer got (its size must be known), and its ticket.
async fn offered(viewer: &Viewer) -> (u32, u32, String, [u8; FILE_TICKET]) {
    let (epoch, file, name, size) = viewer.wait_for("a download offer", |s| s.offers.first().cloned()).await;
    assert!(size.is_some(), "{name} was offered without its size");
    let ticket = viewer.wait_for("its ticket", |s| s.links.iter().find(|l| l.1 == file).map(|l| l.2)).await;
    (epoch, file, name, ticket)
}

/// Clicks at (`x`, `y`) until a download is offered, giving each click three seconds: a click that lands before the
/// page's script runs (or the PDF viewer has loaded) does nothing, and a save Chrome refuses for want of the click's
/// user activation (on a loaded host it can reach Chrome's browser process after the page's request) is tried again
/// at the next.
async fn click_until_offered(person: &Viewer, held: u32, x: u16, y: u16) {
    person.click_until(held, (x, y), 6, |seen| !seen.offers.is_empty()).await;
}

/// The size the first download was offered with.
fn offered_size(viewer: &Viewer) -> Option<u64> {
    viewer.seen.lock().unwrap().offers.first().and_then(|o| o.3)
}

/// Fetches a whole download; the status and the bytes.
async fn fetch(admin: &Admin, session: &str, ticket: &[u8]) -> (u16, Vec<u8>) {
    let response = admin.download(session, ticket).await;
    let status = response.status().as_u16();
    (status, response.bytes().await.map(|b| b.to_vec()).unwrap_or_default())
}

#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_download_reaches_the_holder_byte_exact_and_nobody_else() {
    let admin = Admin::login().await;
    let other = Admin::login().await;
    let (session, person, held) = admin.hold(DOWNLOAD, "Download").await;
    let watcher = admin.watch(&session, 0).await;
    click_until_offered(&person, held, 640, 400).await;
    let (epoch, file, name, ticket) = offered(&person).await;
    assert_eq!((epoch, name.as_str()), (held, "relatório.dat"));
    // Another login cannot use the ticket (and does not spend it).
    assert_eq!(fetch(&other, &session, &ticket).await.0, 404);
    let response = admin.download(&session, &ticket).await;
    assert_eq!(response.status().as_u16(), 200);
    let disposition = response.headers()["content-disposition"].to_str().unwrap().to_owned();
    assert!(disposition.contains("filename*=UTF-8''relat%C3%B3rio.dat"), "{disposition}");
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(offered_size(&person), Some(300_000));
    assert_eq!(response.headers()["content-length"], "300000");
    let received = response.bytes().await.expect("the whole download").to_vec();
    assert!(received == expected(), "the download arrived byte-exact ({} bytes)", received.len());
    assert_eq!(person.wait_for("the end", |s| s.ends.first().copied()).await, (file, Some(300_000)));
    // Single use.
    assert_eq!(fetch(&admin, &session, &ticket).await.0, 404);
    let seen = |v: &Viewer| {
        let s = v.seen.lock().unwrap();
        (s.offers.len() + s.links.len() + s.ends.len(), s.pieces)
    };
    assert_eq!(seen(&watcher), (0, 0), "a watcher got the download");
    assert_eq!(seen(&person).1, 0, "download bytes reached the WebSocket");
    watcher.close().await;
    person.close().await;
    admin.stop(&session).await;
}

/// The tmpfs and shared memory the session's browser container holds now, from its cgroup.
fn shmem(session: &str) -> u64 {
    let stat = common::cgroup_file(&format!("{INSTANCE}-{session}-browser"), "memory.stat");
    stat.lines().find_map(|l| l.strip_prefix("shmem ")).and_then(|v| v.parse().ok()).expect("shmem")
}

/// A session on the big-download page, its holder, and the download's response once the holder clicked.
async fn big(admin: &Admin, size: u64) -> (String, Viewer, u32, u32, reqwest::Response) {
    big_at(admin, &format!("https://fixture.bench/big.html?size={size}")).await
}

/// [`big`], for the big-download page at `url`.
async fn big_at(admin: &Admin, url: &str) -> (String, Viewer, u32, u32, reqwest::Response) {
    let (session, person, held) = admin.hold(url, "Big download").await;
    click_until_offered(&person, held, 640, 400).await;
    let (_, file, name, ticket) = offered(&person).await;
    assert_eq!(name, "big.bin");
    let response = admin.download(&session, &ticket).await;
    assert_eq!(response.status().as_u16(), 200);
    (session, person, held, file, response)
}

/// Reads `response`, checking every byte against the fixture, at most `rate` bytes a second; the bytes read, and
/// whether it ended cleanly (false: it failed).
async fn read(response: &mut reqwest::Response, from: u64, rate: u64, upto: u64) -> (u64, bool) {
    let (started, mut at) = (Instant::now(), from);
    while at < upto {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                for (i, &b) in chunk.iter().enumerate() {
                    assert_eq!(b, big_byte(at + i as u64), "byte {} of the download", at + i as u64);
                }
                at += chunk.len() as u64;
            }
            Ok(None) => return (at, true),
            Err(_) => return (at, false),
        }
        while (at - from) as f64 > started.elapsed().as_secs_f64() * rate as f64 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    (at, true)
}

#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_download_far_larger_than_the_browsers_memory_streams_at_the_persons_pace() {
    const SIZE: u64 = 4_831_838_208; // 4.5 GiB: past 32-bit sizes, and 18 times the browser's downloads tmpfs
    let admin = Admin::login().await;
    let (session, person, _, file, mut response) = big(&admin, SIZE).await;
    // The person's browser knows the total from the start: percentage and time left.
    assert_eq!(offered_size(&person), Some(SIZE));
    assert_eq!(response.headers()["content-length"], SIZE.to_string().as_str());
    let baseline = shmem(&session);
    let peak = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(baseline));
    let sampler = {
        let (peak, session) = (peak.clone(), session.clone());
        tokio::spawn(async move {
            loop {
                peak.fetch_max(shmem(&session), std::sync::atomic::Ordering::Relaxed);
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
    };
    // Far slower than the fixture serves for ten seconds: the browser must wait for the person, not fill its tmpfs.
    let (slow, ok) = read(&mut response, 0, 4 << 20, 40 << 20).await;
    assert!(ok, "the download failed after {slow} bytes while the person read slowly");
    let (at, ok) = read(&mut response, slow, u64::MAX, u64::MAX).await;
    sampler.abort();
    assert!(ok && at == SIZE, "the download ended cleanly at {at} of {SIZE} bytes");
    assert_eq!(person.wait_for("the end", |s| s.ends.first().copied()).await, (file, Some(SIZE)));
    let grew = peak.load(std::sync::atomic::Ordering::Relaxed).saturating_sub(baseline);
    assert!(grew < 160 << 20, "the browser's tmpfs grew by {} MiB", grew >> 20);
    person.close().await;
    admin.stop(&session).await;
}

#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_download_the_person_cancels_fails_and_is_discarded() {
    let admin = Admin::login().await;
    let (session, person, _, file, mut response) = big(&admin, 1 << 30).await;
    let baseline = shmem(&session);
    let (at, ok) = read(&mut response, 0, u64::MAX, 8 << 20).await;
    assert!(ok && at >= 8 << 20);
    drop(response);
    assert_eq!(person.wait_for("the failure", |s| s.ends.first().copied()).await, (file, None));
    // The browser keeps downloading it, and the launcher throws its bytes away.
    for _ in 0..30 {
        let grew = shmem(&session).saturating_sub(baseline);
        assert!(grew < 160 << 20, "the browser's tmpfs grew by {} MiB after the cancel", grew >> 20);
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    person.close().await;
    admin.stop(&session).await;
}

#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_download_whose_connection_drops_fails_and_never_reaches_the_holder_mixed() {
    // The fixture breaks the first connection after 32 MiB and serves other bytes to any later one, which Chromium asks
    // for when it starts the download again on its own: the person gets only the first bytes, then a failure.
    let id = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let url = format!("https://fixture.bench/big.html?size={}&cut={}&id={id}", 1u64 << 30, 32u64 << 20);
    let admin = Admin::login().await;
    let (session, person, _, file, mut response) = big_at(&admin, &url).await;
    let (at, ok) = read(&mut response, 0, u64::MAX, u64::MAX).await;
    assert!(!ok && at <= 32 << 20, "the download ended cleanly at {at} bytes");
    assert_eq!(person.wait_for("the failure", |s| s.ends.first().copied()).await, (file, None));
    // Chromium asks again on its own, which can come after the person heard of the failure.
    let requests = || {
        let (ok, log) = common::docker(&["logs", "easi-run-bench-fixtures-1"]);
        assert!(ok, "{log}");
        log.lines().filter(|line| line.starts_with(&format!("big {id}: request"))).count()
    };
    let asked = common::until!(100, Duration::from_millis(100), Some(requests()).filter(|&n| n >= 2));
    assert!(asked.is_some(), "Chromium asked for the download {} times: it never wrote it again", requests());
    person.close().await;
    admin.stop(&session).await;
}

#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_download_fails_when_control_moves_and_is_not_offered_to_the_next_holder() {
    let admin = Admin::login().await;
    let (session, person, _, file, mut response) = big(&admin, 1 << 30).await;
    let (at, ok) = read(&mut response, 0, u64::MAX, 8 << 20).await;
    assert!(ok && at >= 8 << 20);
    let next = admin.watch(&session, 0).await;
    next.wait_lease(LeaseHolder::Other).await;
    admin.take(&session, &next).await;
    let (_, ok) = read(&mut response, at, u64::MAX, u64::MAX).await;
    assert!(!ok, "the download ended cleanly after control moved");
    tokio::time::sleep(Duration::from_secs(2)).await;
    let reoffered = {
        let s = next.seen.lock().unwrap();
        s.offers.iter().any(|o| o.1 == file) || !s.links.is_empty()
    };
    assert!(!reoffered, "the next holder was offered the download");
    next.close().await;
    person.close().await;
    admin.stop(&session).await;
}

/// The 32-bit FNV-1a hash the upload fixture shows for a file.
fn fnv1a(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0x811c_9dc5u32, |h, &b| (h ^ u32::from(b)).wrapping_mul(0x0100_0193))
}

/// Uploads `bytes` as `name`, within the window the display acknowledges, and waits until the browser stored it. An
/// upload the display failed on the way, which it no longer acknowledges, fails at once.
async fn upload(person: &Viewer, epoch: u32, id: u32, name: &str, bytes: &[u8]) {
    let size = bytes.len() as u64;
    person.send(Message::UploadStart { lease_epoch: epoch, upload: id, size, name: name.into() }).await;
    let mut sent = 0u64;
    while sent < size {
        let (acked, ended) = {
            let seen = person.seen.lock().unwrap();
            let acked = seen.acks.iter().filter(|a| a.0 == id).map(|a| a.1).max().unwrap_or(0);
            (acked, seen.uploaded.iter().find(|u| u.0 == id).map(|u| u.1))
        };
        assert_ne!(ended, Some(false), "the browser stored {name}");
        if sent - acked >= UPLOAD_WINDOW {
            tokio::time::sleep(Duration::from_millis(10)).await;
            continue;
        }
        let end = (sent + UPLOAD_CHUNK as u64).min(size).min(acked + UPLOAD_WINDOW);
        let data = bytes[sent as usize..end as usize].to_vec();
        person.send(Message::UploadData { lease_epoch: epoch, upload: id, offset: sent, data }).await;
        sent = end;
    }
    let stored = person.wait_for("the upload's result", |s| s.uploaded.iter().find(|u| u.0 == id).map(|u| u.1)).await;
    assert!(stored, "the browser stored {name}");
}

#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_file_given_to_the_pages_chooser_or_dragged_and_dropped_on_it_reaches_the_page_byte_exact() {
    let admin = Admin::login().await;
    let (session, person, held) = admin.hold(UPLOAD, "Upload").await;
    let watcher = admin.watch(&session, 0).await;
    // The person drags two files over the page and away again: the page sees them come (with their media types) and
    // go.
    let types = vec!["image/png".to_owned(), "application/pdf".to_owned()];
    for x in [600, 640] {
        person.send(Message::FileDrag { lease_epoch: held, x, y: 400, types: types.clone() }).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    person.send(Message::FileDragEnd { lease_epoch: held }).await;
    // Then drags a file over it and lets go: the page's drag holds through the session's first upload, and a click
    // after it neither drops nor reaches the page's file input; then the file drops.
    let dropped: Vec<u8> = (0..1_572_864u32).map(|i| (i * 7) as u8).collect();
    person.send(Message::FileDrag { lease_epoch: held, x: 640, y: 400, types: vec!["image/png".into()] }).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    upload(&person, held, 1, "foto.png", &dropped).await;
    person.click(held, 640, 400).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(person.seen.lock().unwrap().requests.is_empty(), "a click during the held drag reached the page");
    person.send(Message::FileDrop { lease_epoch: held, x: 640, y: 400, uploads: vec![1] }).await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    // Then clicks the page's file input: the page's chooser asks the person, through the portal and the display.
    person.click(held, 640, 400).await;
    let (epoch, request) = person.wait_for("the file chooser request", |s| s.requests.first().copied()).await;
    assert_eq!(epoch, held);
    let chosen: Vec<u8> = (0..300_000u32).map(|i| (i * 13 + (i >> 9)) as u8).collect();
    upload(&person, held, 2, "relatório.bin", &chosen).await;
    person.send(Message::FileChoose { lease_epoch: held, request, uploads: vec![2] }).await;
    // Two drops back to back both arrive, one after the other, and a drag over the page asked for meanwhile resumes
    // after them.
    upload(&person, held, 3, "a.txt", b"first").await;
    upload(&person, held, 4, "b.txt", b"second").await;
    person.send(Message::FileDrop { lease_epoch: held, x: 640, y: 400, uploads: vec![3] }).await;
    person.send(Message::FileDrop { lease_epoch: held, x: 640, y: 400, uploads: vec![4] }).await;
    person.send(Message::FileDrag { lease_epoch: held, x: 640, y: 400, types: vec!["text/plain".into()] }).await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    person.send(Message::FileDragEnd { lease_epoch: held }).await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert!(watcher.seen.lock().unwrap().requests.is_empty(), "a watcher was asked for files");
    admin.hand_back(&session, &person).await;
    let text = page_text(&admin, &session).await;
    let expect = |how: &str, name: &str, bytes: &[u8]| format!("{how} {name} {} {:08x}", bytes.len(), fnv1a(bytes));
    assert!(text.contains(&expect("chosen", "relatório.bin", &chosen)), "the chooser's file: {text}");
    assert!(text.contains(&expect("dropped", "foto.png", &dropped)), "the dropped file: {text}");
    assert!(text.contains(&expect("dropped", "a.txt", b"first")), "the first of two drops: {text}");
    assert!(text.contains(&expect("dropped", "b.txt", b"second")), "the second of two drops: {text}");
    let lines = |line: &str| text.matches(&format!("static = \"{line}\"")).count();
    assert_eq!(lines("entered Files file:image/png,file:application/pdf"), 1, "the drag over the page: {text}");
    // The drag held while the file uploaded, and the drop's own drag.
    assert_eq!(lines("entered Files file:image/png"), 2, "the held drag: {text}");
    // The two drops' own drags, and the drag that waited for them.
    assert_eq!(lines("entered Files file:text/plain"), 3, "the drag resumed after the drops: {text}");
    assert_eq!(lines("drag left"), 3, "each drag over the page left it or gave way to its drop: {text}");
    assert_eq!(text.matches("static = \"dropped ").count(), 3, "the uploaded files dropped, nothing else: {text}");
    watcher.close().await;
    person.close().await;
    admin.stop(&session).await;
}

/// A file dropped on a page's drop zone (only the zone takes it, as image tools have it, while the page animates)
/// reaches the page, not a tab of its own that Chrome opens it in.
#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_file_dropped_on_a_busy_pages_drop_zone_reaches_the_page() {
    let admin = Admin::login().await;
    let (session, person, held) = admin.hold(DROP_ZONE, "Drop zone").await;
    let photo: Vec<u8> = (0..200_000u32).map(|i| (i * 11) as u8).collect();
    person.send(Message::FileDrag { lease_epoch: held, x: 640, y: 400, types: vec!["image/png".into()] }).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    upload(&person, held, 1, "praia.png", &photo).await;
    person.send(Message::FileDrop { lease_epoch: held, x: 640, y: 400, uploads: vec![1] }).await;
    // Control given back ends the drop if the browser has not read it yet, and nothing says when it has: time enough
    // for it on a host running every suite at once (3 s fell short once).
    tokio::time::sleep(Duration::from_secs(6)).await;
    admin.hand_back(&session, &person).await;
    // The page says so once it has read the file: asked until it does.
    let expect = format!("dropped praia.png {} {:08x}", photo.len(), fnv1a(&photo));
    let mut text = String::new();
    for _ in 0..30 {
        text = page_text(&admin, &session).await;
        if text.contains(&expect) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert!(text.contains(&expect), "the dropped file: {text}");
    person.close().await;
    admin.stop(&session).await;
}

#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn an_element_dragged_within_the_page_drops_where_the_person_lets_go() {
    let admin = Admin::login().await;
    let (session, person, first) = admin.hold("https://fixture.bench/dnd.html", "Drag and drop").await;
    // The person leaves the middle button held and control changes hands: the next holder starts with every button up.
    use display_protocol::PointerKind;
    person.send(Message::Pointer { lease_epoch: first, kind: PointerKind::Down, button: 1, x: 600, y: 650 }).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    admin.hand_back(&session, &person).await;
    let held = admin.take(&session, &person).await;
    // The person presses on the element, moves in steps past the drag threshold to the zone, and lets go there: the
    // browser starts its own drag, which the display carries with the pointer.
    let pointer = |kind, x| person.pointer(held, kind, x, 440);
    pointer(PointerKind::Move, 230).await;
    pointer(PointerKind::Down, 230).await;
    for x in (240..=1000).step_by(40) {
        tokio::time::sleep(Duration::from_millis(30)).await;
        pointer(PointerKind::Move, x).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    pointer(PointerKind::Up, 1000).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    admin.hand_back(&session, &person).await;
    let text = page_text(&admin, &session).await;
    for step in ["started", "over the zone", "dropped logo"] {
        assert!(text.contains(&format!("static = \"{step}\"")), "{step}: {text}");
    }
    person.close().await;
    admin.stop(&session).await;
}

#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_file_the_page_saves_reaches_the_holder_byte_exact() {
    let admin = Admin::login().await;
    let (session, person, held) = admin.hold(SAVE, "Save").await;
    // The page's save dialog goes through the portal's SaveFile, as the PDF viewer's download button does.
    click_until_offered(&person, held, 640, 400).await;
    let (_, _, name, ticket) = offered(&person).await;
    assert_eq!(name, "notas.txt");
    let (status, received) = fetch(&admin, &session, &ticket).await;
    assert_eq!(status, 200);
    let expected = format!("Notas salvas: {}", "ação ".repeat(1000));
    assert!(received == expected.as_bytes(), "the saved file arrived byte-exact ({} bytes)", received.len());
    person.close().await;
    admin.stop(&session).await;
}

/// Chromium's own "Save as" of a page (Ctrl+S, or the page's context menu) saves it as one file, MHTML, and that file
/// reaches the holder whole: its closing boundary last, and the page's text in it.
#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_page_saved_with_chromes_save_as_reaches_the_holder_as_one_file() {
    use display_protocol::keys::{LEFTCTRL, S};
    let admin = Admin::login().await;
    let (session, person, held) = admin.hold(ARTICLE, "Static article").await;
    // Pressed again until a file is offered: a press that lands before the page has focus saves nothing.
    for _ in 0..4 {
        person.chord(held, &[LEFTCTRL, S]).await;
        let pressed = Instant::now();
        while pressed.elapsed() < Duration::from_secs(5) && person.seen.lock().unwrap().offers.is_empty() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        if !person.seen.lock().unwrap().offers.is_empty() {
            break;
        }
    }
    let (_, _, name, ticket) = offered(&person).await;
    assert_eq!(name, "Static article.mhtml");
    let (status, received) = fetch(&admin, &session, &ticket).await;
    let text = String::from_utf8_lossy(&received);
    let boundary = text.split("boundary=\"").nth(1).and_then(|b| b.split('"').next()).expect("an MHTML header");
    assert!(status == 200 && text.trim_end().ends_with(&format!("--{boundary}--")), "{status}: {} bytes", text.len());
    assert!(text.contains("Static article"), "the page is in it");
    person.close().await;
    admin.stop(&session).await;
}

#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn the_pdf_viewers_download_button_reaches_the_holder() {
    let admin = Admin::login().await;
    let session = admin.create("https://fixture.bench/sample.pdf").await;
    // A fresh session: the viewer opens holding control.
    let person = admin.watch(&session, 0).await;
    let held = person.wait_lease(LeaseHolder::You).await;
    // The viewer's download button, at the right of its toolbar on a 1280-pixel screen.
    click_until_offered(&person, held, 1176, 28 + common::PAGE_TOP).await;
    let (_, _, name, ticket) = offered(&person).await;
    let expected = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../bench/fixtures/sample.pdf")).unwrap();
    let (status, received) = fetch(&admin, &session, &ticket).await;
    assert!(name.ends_with(".pdf") && status == 200 && received == expected, "the PDF arrived: {name} ({status})");
    person.close().await;
    admin.stop(&session).await;
}
