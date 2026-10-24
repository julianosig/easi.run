//! The browser's sound (docs/design.md §3.3): a page's tone reaches a viewer as Opus that decodes, a silent page sends
//! no sound, and a writer flooding the display's FIFO costs the display bounded work and is not heard. Runs only with `DISPLAY_E2E=1`
//! against the bench profile (see `common`); the sound decodes with ffmpeg in the `easi-run/tools:dev` image.

mod common;

use std::time::{Duration, Instant};

use common::{Admin, Viewer, enabled};
use display_protocol::{AUDIO_FRAME_US, LeaseHolder};

const TONE: &str = "https://fixture.bench/tone.html";
const ARTICLE: &str = "https://fixture.bench/article.html";

/// The Ogg CRC-32 (polynomial 0x04c11db7, no reflection).
fn ogg_crc(data: &[u8]) -> u32 {
    let mut crc = 0u32;
    for &byte in data {
        crc ^= u32::from(byte) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 { (crc << 1) ^ 0x04c1_1db7 } else { crc << 1 };
        }
    }
    crc
}

fn ogg_page(out: &mut Vec<u8>, packet: &[u8], flags: u8, granule: u64, sequence: u32) {
    let mut lacing = vec![255u8; packet.len() / 255];
    lacing.push((packet.len() % 255) as u8);
    let start = out.len();
    out.extend_from_slice(b"OggS\0");
    out.push(flags);
    out.extend_from_slice(&granule.to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&sequence.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.push(lacing.len() as u8);
    out.extend_from_slice(&lacing);
    out.extend_from_slice(packet);
    let crc = ogg_crc(&out[start..]);
    out[start + 22..start + 26].copy_from_slice(&crc.to_le_bytes());
}

/// The packets as an Ogg Opus file (RFC 7845), one packet per page.
fn ogg_opus(packets: &[Vec<u8>]) -> Vec<u8> {
    let mut head = b"OpusHead".to_vec();
    head.extend_from_slice(&[1, 2]);
    head.extend_from_slice(&312u16.to_le_bytes());
    head.extend_from_slice(&48_000u32.to_le_bytes());
    head.extend_from_slice(&[0, 0, 0]);
    let mut tags = b"OpusTags".to_vec();
    tags.extend_from_slice(&4u32.to_le_bytes());
    tags.extend_from_slice(b"test");
    tags.extend_from_slice(&0u32.to_le_bytes());
    let mut out = Vec::new();
    ogg_page(&mut out, &head, 0x02, 0, 0);
    ogg_page(&mut out, &tags, 0, 0, 1);
    for (i, packet) in packets.iter().enumerate() {
        let last = i + 1 == packets.len();
        ogg_page(&mut out, packet, if last { 0x04 } else { 0 }, (i as u64 + 1) * 960, i as u32 + 2);
    }
    out
}

/// Runs ffmpeg on `dir`'s `in.opus` with `args`; whether it succeeded, and what it printed on standard error.
fn ffmpeg(dir: &std::path::Path, args: &[&str]) -> (bool, String) {
    let output = common::tools(dir, &[&["ffmpeg", "-nostdin", "-i", "/d/in.opus"][..], args].concat());
    (output.status.success(), String::from_utf8_lossy(&output.stderr).into_owned())
}

/// Decodes the packets, failing on any decoder error, and returns the overall RMS level in dB.
fn decoded_level(packets: &[Vec<u8>], name: &str) -> f64 {
    let dir = common::scratch(&format!("audio-{name}"));
    std::fs::write(dir.join("in.opus"), ogg_opus(packets)).unwrap();
    let (ok, errors) = ffmpeg(&dir, &["-v", "error", "-f", "null", "-"]);
    assert!(ok && errors.trim().is_empty(), "{name}: the sound does not decode cleanly: {errors}");
    let (ok, stats) = ffmpeg(&dir, &["-af", "astats", "-f", "null", "-"]);
    assert!(ok, "{name}: ffmpeg failed: {stats}");
    let overall = stats.split("Overall").nth(1).expect("astats prints an overall section");
    overall
        .lines()
        .find_map(|l| l.split("RMS level dB:").nth(1))
        .and_then(|v| v.trim().parse().ok())
        .expect("an overall RMS level")
}

/// Microseconds of CPU the container has used.
fn cpu_us(container: &str) -> u64 {
    let stat = common::cgroup_file(container, "cpu.stat");
    stat.lines().find_map(|l| l.strip_prefix("usage_usec ")).unwrap().parse().unwrap()
}

#[tokio::test]
async fn a_tone_in_the_page_reaches_the_viewer_as_opus_and_stops_after_it() {
    if !enabled() {
        return;
    }
    let admin = Admin::login().await;
    let session = admin.create(TONE, false).await;
    let viewer = admin.watch(&session, 0).await;
    admin.wait_title(&session, "Tone").await;
    assert!(viewer.audio().is_empty(), "the page sent sound before it played any");
    // A page may start sound only after a gesture: the person clicks it.
    admin.take_control(&session, &viewer).await.expect("control");
    let epoch = viewer.wait_lease(LeaseHolder::You).await;
    click_until_sound(&viewer, epoch, "tone").await;
    let clicked = Instant::now();
    while viewer.audio().len() < 40 && clicked.elapsed() < Duration::from_secs(10) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    // The tone lasts 1.5 s; well after it, the sound must have stopped.
    tokio::time::sleep(Duration::from_secs(4)).await;
    let audio = viewer.audio();
    let end = Instant::now();
    assert!((50..=120).contains(&audio.len()), "{} packets for a 1.5 s tone", audio.len());
    let (last_arrival, _, _) = audio.last().unwrap();
    assert!(end - *last_arrival >= Duration::from_secs(2), "packets kept coming after the tone ended");
    // One stream: consecutive timestamps, a packet's duration apart.
    let restarts = audio.windows(2).filter(|w| w[1].1 != w[0].1 + AUDIO_FRAME_US).count();
    assert!(restarts <= 1, "{restarts} gaps in one tone");
    let packets: Vec<Vec<u8>> = audio.into_iter().map(|(_, _, p)| p).collect();
    let level = decoded_level(&packets, "tone");
    // A sine at gain 0.3 is about -13.5 dB RMS; the silent tail lowers the average a little.
    assert!(level > -20.0, "the tone decoded at {level:.1} dB");
    viewer.close().await;
    admin.stop(&session).await;
}

/// The period, in microseconds, over which a running container's CPU quota is measured (its cgroup's `cpu.max`).
fn cpu_period_us(container: &str) -> u64 {
    common::cgroup_file(container, "cpu.max").split_whitespace().nth(1).unwrap().parse().unwrap()
}

/// A tone that starts while the page keeps the browser's CPUs busy (as a video page does while it starts) reaches the
/// viewer as one unbroken stream: no restart, every packet's timestamp right after the last one's.
#[tokio::test]
async fn a_tone_under_a_busy_start_reaches_the_viewer_unbroken() {
    if !enabled() {
        return;
    }
    let (admin, session, viewer) = play("busy-tone", "Busy tone").await;
    // The page's workers spend the browser's quota early in each period; the container then pauses until the next, so
    // the period must be short for the sound to go on.
    for role in ["browser", "display"] {
        assert_eq!(cpu_period_us(&format!("easirun-{session}-{role}")), 10_000, "{role}");
    }
    unbroken(&admin, &session, viewer).await;
}

/// A tone while the whole screen changes every 300 ms (as when a page or a video changes) reaches the viewer as one
/// unbroken stream: large frames of media waiting for the viewer never hold its sound back.
#[tokio::test]
async fn a_tone_under_large_screen_changes_reaches_the_viewer_unbroken() {
    if !enabled() {
        return;
    }
    let (admin, session, viewer) = play("noisy-tone", "Noisy tone").await;
    unbroken(&admin, &session, viewer).await;
}

/// Clicks the fixture `page` until its sound starts. A click as the window first shows may reach Chrome before the
/// page takes input: then, with no sound yet, it clicks again (each tone page plays only once).
async fn click_until_sound(viewer: &Viewer, epoch: u32, page: &str) {
    for _ in 0..5 {
        viewer.click(epoch, 640, 400).await;
        let clicked = Instant::now();
        while viewer.audio().is_empty() && clicked.elapsed() < Duration::from_secs(3) {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        if !viewer.audio().is_empty() {
            return;
        }
    }
    panic!("{page} never took the click");
}

/// Opens the fixture `page`, waits for its `title`, and clicks it until its 6 s tone starts.
async fn play(page: &str, title: &str) -> (Admin, String, Viewer) {
    let admin = Admin::login().await;
    let session = admin.create(&format!("https://fixture.bench/{page}.html"), false).await;
    let viewer = admin.watch(&session, 0).await;
    admin.wait_title(&session, title).await;
    admin.take_control(&session, &viewer).await.expect("control");
    let epoch = viewer.wait_lease(LeaseHolder::You).await;
    click_until_sound(&viewer, epoch, page).await;
    (admin, session, viewer)
}

/// The 6 s tone reaches `viewer` whole: nearly every packet, every timestamp right after the last one's.
async fn unbroken(admin: &Admin, session: &str, viewer: Viewer) {
    tokio::time::sleep(Duration::from_secs(8)).await;
    let audio = viewer.audio();
    let restarts: Vec<usize> =
        audio.windows(2).enumerate().filter(|(_, w)| w[1].1 != w[0].1 + AUDIO_FRAME_US).map(|(i, _)| i + 1).collect();
    let gaps: Vec<u128> = audio.windows(2).map(|w| (w[1].0 - w[0].0).as_millis()).filter(|&ms| ms > 100).collect();
    assert!(audio.len() >= 290, "{} packets for a 6 s tone", audio.len());
    assert!(
        restarts.is_empty(),
        "the stream restarted at packets {restarts:?} of {}; arrival gaps over 100 ms: {gaps:?}",
        audio.len()
    );
    viewer.close().await;
    admin.stop(session).await;
}

#[tokio::test]
async fn a_silent_page_sends_no_sound() {
    if !enabled() {
        return;
    }
    let admin = Admin::login().await;
    let session = admin.create(ARTICLE, false).await;
    let viewer = admin.watch(&session, 0).await;
    // Quiet for four seconds once the page has loaded (before that, any page is silent).
    admin.wait_title(&session, "Static article").await;
    tokio::time::sleep(Duration::from_secs(4)).await;
    assert!(viewer.seen.lock().unwrap().configs >= 1, "the viewer was configured");
    assert!(viewer.audio().is_empty(), "a silent page sent {} sound packets", viewer.audio().len());
    viewer.close().await;
    admin.stop(&session).await;
}

#[tokio::test]
async fn a_writer_flooding_the_fifo_costs_the_display_bounded_work_and_is_not_heard() {
    if !enabled() {
        return;
    }
    let admin = Admin::login().await;
    let session = admin.create(ARTICLE, false).await;
    let viewer = admin.watch(&session, 0).await;
    // The page's own load, and what it draws, stay outside the measured window.
    admin.wait_title(&session, "Static article").await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let display = format!("easirun-{session}-display");
    let before = cpu_us(&display);
    let started = Instant::now();
    // As the browser's uid and group, with the volume read-only as the browser mounts it: grow the pipe to the most an
    // unprivileged writer may, then write random bytes as fast as possible for five seconds.
    let flood = r#"open(F, ">", "/v/audio") or die "open: $!"; fcntl(F, 1031, 1048576) or die "resize: $!";
        open(R, "<", "/dev/urandom") or die; my $end = time + 5;
        while (time < $end) { sysread(R, my $b, 65536); syswrite(F, $b); }"#;
    let volume = format!("easirun-{session}-wayland:/v:ro");
    let (ok, text) = common::docker(
        &[
            &["run", "--rm", "--pull", "never", "--network", "none", "--read-only", "-u", "10001:10011", "-v"][..],
            &[&volume],
            &["--entrypoint", "perl", "easi-run/tools:dev", "-e", flood],
        ]
        .concat(),
    );
    assert!(ok, "the flooding writer ran: {text}");
    let seconds = started.elapsed().as_secs_f64();
    let cpu = (cpu_us(&display) - before) as f64 / 1e6 / seconds;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let packets = viewer.audio().len() as f64;
    assert!(cpu < 0.3, "the display used {cpu:.2} CPU while the FIFO was flooded");
    // At most the first turn's sound, before the backlog builds; never a stream of noise (real time is 50 a second).
    assert!(packets <= 5.0, "{packets} packets of noise in {seconds:.1} s");
    // A writer faster than real time keeps the FIFO backlogged, so its sound is discarded rather than heard; the
    // display and the viewer's stream carry on.
    assert!(!viewer.seen.lock().unwrap().closed, "the viewer's stream ended");
    let (_, running) = common::docker(&["inspect", "-f", "{{.State.Running}}", &display]);
    assert_eq!(running.trim(), "true", "the display stopped");
    viewer.close().await;
    admin.stop(&session).await;
}
