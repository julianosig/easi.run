//! Files between the person and the browser (docs/design.md §3.6): a download reaches the person holding control,
//! byte-exact, through the single-use ticket of its native download, and nobody else; one far larger than the browser's
//! memory streams at the person's pace; a cancelled one or one whose lease moves ends as a failure; a file the person
//! gives the page's file chooser, or drops on the page, reaches the page byte-exact. Ignored by default: scripts/e2e.sh
//! runs it against the bench profile.

mod common;

use std::time::{Duration, Instant};

use common::{Admin, INSTANCE, Viewer, page_text};
use display_protocol::{FILE_TICKET, LeaseHolder, Message, UPLOAD_CHUNK, UPLOAD_WINDOW};

const DOWNLOAD: &str = "https://fixture.bench/download.html";
const UPLOAD: &str = "https://fixture.bench/upload.html";
const DROP_ZONE: &str = "https://fixture.bench/drop.html";
const SAVE: &str = "https://fixture.bench/save.html";

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
/// page's script runs (or the PDF viewer has loaded) does nothing.
async fn click_until_offered(person: &Viewer, held: u32, x: u16, y: u16) {
    for _ in 0..6 {
        person.click(held, x, y).await;
        let clicked = Instant::now();
        while clicked.elapsed() < Duration::from_secs(3) {
            if !person.seen.lock().unwrap().offers.is_empty() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
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
    let session = admin.create(DOWNLOAD, false).await;
    let person = admin.watch(&session, 0).await;
    let watcher = admin.watch(&session, 0).await;
    admin.wait_title(&session, "Download").await;
    admin.take_control(&session, &person).await.expect("control");
    let held = person.wait_lease(LeaseHolder::You).await;
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

/// Byte `i` of the fixture's `/big` downloads.
fn big_byte(i: u64) -> u8 {
    (i.wrapping_mul(7) ^ (i >> 8) ^ (i >> 24)) as u8
}

/// The tmpfs and shared memory the session's browser container holds now, from its cgroup.
fn shmem(session: &str) -> u64 {
    let stat = common::cgroup_file(&format!("{INSTANCE}-{session}-browser"), "memory.stat");
    stat.lines().find_map(|l| l.strip_prefix("shmem ")).and_then(|v| v.parse().ok()).expect("shmem")
}

/// A session on the big-download page, its holder, and the download's response once the holder clicked.
async fn big(admin: &Admin, size: u64) -> (String, Viewer, u32, u32, reqwest::Response) {
    let session = admin.create(&format!("https://fixture.bench/big.html?size={size}"), false).await;
    let person = admin.watch(&session, 0).await;
    admin.wait_title(&session, "Big download").await;
    admin.take_control(&session, &person).await.expect("control");
    let held = person.wait_lease(LeaseHolder::You).await;
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
async fn a_download_fails_when_control_moves_and_is_not_offered_to_the_next_holder() {
    let admin = Admin::login().await;
    let (session, person, _, file, mut response) = big(&admin, 1 << 30).await;
    let (at, ok) = read(&mut response, 0, u64::MAX, 8 << 20).await;
    assert!(ok && at >= 8 << 20);
    let next = admin.watch(&session, 0).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    admin.take_control(&session, &next).await.expect("control");
    next.wait_lease(LeaseHolder::You).await;
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

/// Uploads `bytes` as `name`, within the window the display acknowledges, and waits until the browser stored it.
async fn upload(person: &Viewer, epoch: u32, id: u32, name: &str, bytes: &[u8]) {
    let size = bytes.len() as u64;
    person.send(Message::UploadStart { lease_epoch: epoch, upload: id, size, name: name.into() }).await;
    let mut sent = 0u64;
    while sent < size {
        let acked = person.seen.lock().unwrap().acks.iter().filter(|a| a.0 == id).map(|a| a.1).max().unwrap_or(0);
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

/// What the page shows, through the agent's accessibility tree (read once the person gave control back).
#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_file_given_to_the_pages_chooser_or_dragged_and_dropped_on_it_reaches_the_page_byte_exact() {
    let admin = Admin::login().await;
    let session = admin.create(UPLOAD, false).await;
    let person = admin.watch(&session, 0).await;
    let watcher = admin.watch(&session, 0).await;
    admin.wait_title(&session, "Upload").await;
    admin.take_control(&session, &person).await.expect("control");
    let held = person.wait_lease(LeaseHolder::You).await;
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
    admin.give_back(&session, &person).await.expect("give back");
    person.wait_lease(LeaseHolder::Agent).await;
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
    let session = admin.create(DROP_ZONE, false).await;
    let person = admin.watch(&session, 0).await;
    admin.wait_title(&session, "Drop zone").await;
    admin.take_control(&session, &person).await.expect("control");
    let held = person.wait_lease(LeaseHolder::You).await;
    let photo: Vec<u8> = (0..200_000u32).map(|i| (i * 11) as u8).collect();
    person.send(Message::FileDrag { lease_epoch: held, x: 640, y: 400, types: vec!["image/png".into()] }).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    upload(&person, held, 1, "praia.png", &photo).await;
    person.send(Message::FileDrop { lease_epoch: held, x: 640, y: 400, uploads: vec![1] }).await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    admin.give_back(&session, &person).await.expect("give back");
    person.wait_lease(LeaseHolder::Agent).await;
    let text = page_text(&admin, &session).await;
    let expect = format!("dropped praia.png {} {:08x}", photo.len(), fnv1a(&photo));
    assert!(text.contains(&expect), "the dropped file: {text}");
    person.close().await;
    admin.stop(&session).await;
}

#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn an_element_dragged_within_the_page_drops_where_the_person_lets_go() {
    let admin = Admin::login().await;
    let session = admin.create("https://fixture.bench/dnd.html", false).await;
    let person = admin.watch(&session, 0).await;
    admin.wait_title(&session, "Drag and drop").await;
    admin.take_control(&session, &person).await.expect("control");
    let first = person.wait_lease(LeaseHolder::You).await;
    // The person leaves the middle button held and control changes hands: the next holder starts with every button up.
    use display_protocol::PointerKind;
    person.send(Message::Pointer { lease_epoch: first, kind: PointerKind::Down, button: 1, x: 600, y: 650 }).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    admin.give_back(&session, &person).await.expect("give back");
    person.wait_lease(LeaseHolder::Agent).await;
    admin.take_control(&session, &person).await.expect("control again");
    let held = person.wait_lease(LeaseHolder::You).await;
    // The person presses on the element, moves in steps past the drag threshold to the zone, and lets go there: the
    // browser starts its own drag, which the display carries with the pointer.
    let pointer = |kind, x| Message::Pointer { lease_epoch: held, kind, button: 0, x, y: 440 };
    person.send(pointer(PointerKind::Move, 230)).await;
    person.send(pointer(PointerKind::Down, 230)).await;
    for x in (240..=1000).step_by(40) {
        tokio::time::sleep(Duration::from_millis(30)).await;
        person.send(pointer(PointerKind::Move, x)).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    person.send(pointer(PointerKind::Up, 1000)).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    admin.give_back(&session, &person).await.expect("give back");
    person.wait_lease(LeaseHolder::Agent).await;
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
    let session = admin.create(SAVE, false).await;
    let person = admin.watch(&session, 0).await;
    admin.wait_title(&session, "Save").await;
    admin.take_control(&session, &person).await.expect("control");
    let held = person.wait_lease(LeaseHolder::You).await;
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

#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn the_pdf_viewers_download_button_reaches_the_holder() {
    let admin = Admin::login().await;
    let session = admin.create("https://fixture.bench/sample.pdf", false).await;
    let person = admin.watch(&session, 0).await;
    person.wait_lease(LeaseHolder::Agent).await;
    admin.take_control(&session, &person).await.expect("control");
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
