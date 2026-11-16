//! The ordered hybrid stream's recovery (docs/design.md §3.3), through the running admin application and real Chromium
//! frames. The video checks decode with ffmpeg in the `easi-run/tools:dev` image (bench/tools).

mod common;

use std::time::{Duration, Instant};

use common::{Admin, Viewer, uncovered};
use display_protocol::{CODEC_AV1, CODEC_H264, Codec, Message, Rect};
use serde_json::json;

const SCROLL: &str = "https://fixture.bench/scroll.html";

/// Decodes a video stream (`format` is ffmpeg's `h264` or `obu`) and returns the decoded frame count; any decoder
/// error fails.
fn decodes(stream: &[u8], name: &str, format: &str) -> usize {
    let dir = common::scratch(&format!("stream-{name}"));
    std::fs::write(dir.join("in"), stream).unwrap();
    let count = ["-count_frames", "-select_streams", "v:0", "-show_entries", "stream=nb_read_frames", "-of", "csv=p=0"];
    let output = common::tools(&dir, &[&["ffprobe", "-v", "error", "-f", format][..], &count, &["/d/in"]].concat());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success() && stderr.trim().is_empty(),
        "{name}: the stream does not decode cleanly: {stderr}"
    );
    String::from_utf8_lossy(&output.stdout).trim().parse().unwrap_or(0)
}

fn videos(log: &[(Instant, Message)]) -> Vec<(Instant, u32, u64, bool)> {
    log.iter()
        .filter_map(|(t, m)| match m {
            Message::Video { epoch, revision, keyframe, .. } => Some((*t, *epoch, *revision, *keyframe)),
            _ => None,
        })
        .collect()
}

/// How many video frames `log` holds.
fn video_count(log: &[(Instant, Message)]) -> usize {
    log.iter().filter(|(_, m)| matches!(m, Message::Video { .. })).count()
}

/// Whether the lossless tiles after the last video frame in `log` (the refresh that ends video, split over messages
/// of newer revisions as the pacing allows) cover every pixel of that frame's rectangle.
fn final_refresh(log: &[(Instant, Message)]) -> Option<bool> {
    let (_, last) = log.iter().rev().find(|(_, m)| matches!(m, Message::Video { .. }))?;
    let Message::Video { revision: last_video, rect, .. } = last else { unreachable!() };
    let tiles = log.iter().flat_map(|(_, m)| match m {
        Message::Tiles { revision, rects, .. } if revision > last_video => &rects[..],
        _ => &[],
    });
    Some(uncovered(rect, tiles) == 0)
}

/// Revisions never decrease and epochs never go back, in arrival order.
fn assert_ordered(log: &[(Instant, Message)]) {
    let (mut epoch, mut revision) = (0u32, 0u64);
    for (_, m) in log {
        match m {
            Message::Config { epoch: e, .. } => {
                assert!(*e > epoch, "a configuration did not start a newer epoch");
                epoch = *e;
            }
            Message::Video { epoch: e, revision: r, .. }
            | Message::Tiles { epoch: e, revision: r, .. }
            | Message::Moved { epoch: e, revision: r, .. } => {
                assert_eq!(*e, epoch, "media of another epoch than the current configuration");
                assert!(*r >= revision, "a revision went back ({r} after {revision})");
                revision = *r;
            }
            _ => {}
        }
    }
}

async fn settings(admin: &Admin, session: &str, codec: &str, bitrate: u32) {
    let body = json!({"codec": codec, "bitrate_kbps": bitrate, "max_fps": 60});
    admin.call(reqwest::Method::PUT, &format!("/api/sessions/{session}/settings"), Some(body)).await.expect("settings");
}

/// The viewer's log once `done` holds of it, or once `timeout` passed.
async fn log_until(
    viewer: &Viewer,
    timeout: Duration,
    done: impl Fn(&[(Instant, Message)]) -> bool,
) -> Vec<(Instant, Message)> {
    let deadline = Instant::now() + timeout;
    loop {
        {
            // Looked at where it is, and copied once: it holds every picture that came.
            let seen = viewer.seen.lock().unwrap();
            if done(&seen.log) || Instant::now() >= deadline {
                return seen.log.clone();
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn wait_video(viewer: &Viewer, at_least: usize) {
    let log = log_until(viewer, Duration::from_secs(15), |log| video_count(log) >= at_least).await;
    assert!(videos(&log).len() >= at_least, "no video arrived (is the page moving?)");
}

#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn video_starts_with_a_keyframe_and_ends_with_a_newer_lossless_refresh() {
    let admin = Admin::login().await;
    let session = admin.create(&format!("{SCROLL}?seconds=6")).await;
    let viewer = admin.watch_leaving_control(&session, CODEC_AV1).await;
    // The six seconds of scrolling start as the page loads; then the stream leaves video.
    admin.wait_title(&session, "Smooth scroll").await;
    let log = log_until(&viewer, Duration::from_secs(30), |log| {
        let quiet = videos(log).last().is_some_and(|v| v.0.elapsed() >= Duration::from_secs(1));
        quiet && final_refresh(log) == Some(true)
    })
    .await;
    assert_ordered(&log);
    let video = videos(&log);
    assert!(video.len() >= 30, "scrolling became video ({} frames)", video.len());
    assert!(video[0].3, "video starts with a keyframe");
    let refresh = final_refresh(&log).expect("leaving video ends with lossless tiles at a newer revision");
    assert!(refresh, "the final refresh covers the video's whole region");
    let frames = decodes(&viewer.video(), "transition", "obu");
    assert_eq!(frames, video.len());
    viewer.close().await;
    admin.stop(&session).await;
}

#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_slow_viewer_degrades_and_recovers_without_slowing_a_fast_one() {
    let admin = Admin::login().await;
    let session = admin.create(SCROLL).await;
    // A high bitrate fills the paused viewer's buffers quickly.
    settings(&admin, &session, "av1", 20_000).await;
    let fast = admin.watch(&session, CODEC_AV1).await;
    let slow = admin.watch(&session, CODEC_AV1).await;
    wait_video(&fast, 30).await;
    wait_video(&slow, 30).await;
    let before = fast.look(video_count);
    let start = Instant::now();
    tokio::time::sleep(Duration::from_secs(3)).await;
    let fast_rate = (fast.look(video_count) - before) as f64 / start.elapsed().as_secs_f64();

    slow.paused.store(true, std::sync::atomic::Ordering::Relaxed);
    let paused_at = Instant::now();
    let fast_before = fast.look(video_count);
    tokio::time::sleep(Duration::from_secs(12)).await;
    let fast_while = (fast.look(video_count) - fast_before) as f64 / paused_at.elapsed().as_secs_f64();
    slow.paused.store(false, std::sync::atomic::Ordering::Relaxed);
    let slow_before = slow.look(video_count);
    let resumed = log_until(&slow, Duration::from_secs(5), |log| video_count(log) >= slow_before + 10).await;
    let slow_after = videos(&resumed).len() - slow_before;

    assert!(
        fast_while >= fast_rate * 0.7,
        "the fast viewer kept its rate: {fast_rate:.1} fps before, {fast_while:.1} fps while the other was stalled"
    );
    assert!(slow_after >= 10, "the slow viewer recovered ({slow_after} frames after resuming)");
    slow.look(assert_ordered);
    fast.look(assert_ordered);
    // Frames were skipped before encoding, so the slow viewer's stream still decodes without a keyframe request.
    let decoded = decodes(&slow.video(), "slow", "obu");
    assert_eq!(decoded, slow.look(video_count));
    decodes(&fast.video(), "fast", "obu");
    fast.close().await;
    slow.close().await;
    admin.stop(&session).await;
}

#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_reconnect_mid_stream_starts_from_a_full_picture_and_a_keyframe() {
    let admin = Admin::login().await;
    let session = admin.create(SCROLL).await;
    let first = admin.watch(&session, CODEC_AV1).await;
    wait_video(&first, 20).await;
    first.close().await;
    let again = admin.watch(&session, CODEC_AV1).await;
    wait_video(&again, 20).await;
    let log = again.log();
    assert!(matches!(log[0].1, Message::Config { .. }), "a reconnect starts with a configuration");
    assert!(videos(&log)[0].3, "and its first video frame is a keyframe");
    let Message::Config { width, height, .. } = log[0].1 else { unreachable!() };
    let pictures = log.iter().flat_map(|(_, m)| match m {
        Message::Tiles { rects, .. } => &rects[..],
        Message::Video { rect, keyframe: true, .. } => std::slice::from_ref(rect),
        _ => &[],
    });
    let screen = Rect { x: 0, y: 0, w: width, h: height };
    assert_eq!(uncovered(&screen, pictures), 0, "its tiles and keyframes drew the whole screen");
    assert_ordered(&log);
    decodes(&again.video(), "reconnect", "obu");
    again.close().await;
    admin.stop(&session).await;
}

#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn an_epoch_change_mid_stream_restarts_cleanly() {
    let admin = Admin::login().await;
    let session = admin.create(SCROLL).await;
    let viewer = admin.watch(&session, CODEC_AV1).await;
    wait_video(&viewer, 20).await;
    settings(&admin, &session, "tiles_only", 2500).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    settings(&admin, &session, "av1", 2500).await;
    let configs = |log: &[(Instant, Message)]| -> Vec<u32> {
        log.iter()
            .filter_map(|(_, m)| if let Message::Config { epoch, .. } = m { Some(*epoch) } else { None })
            .collect()
    };
    // Video resumes in the AV1 epoch the last change made.
    let log = log_until(&viewer, Duration::from_secs(15), |log| {
        let epochs = configs(log);
        epochs.len() >= 3 && videos(log).iter().any(|v| Some(&v.1) == epochs.last())
    })
    .await;
    assert_ordered(&log);
    let configs = configs(&log);
    assert!(configs.len() >= 3, "two settings changes made two new epochs: {configs:?}");
    let last = configs[configs.len() - 1];
    let resumed = videos(&log).into_iter().find(|v| v.1 == last).expect("video resumed in the last epoch");
    assert!(resumed.3, "the last epoch started its video with a keyframe");
    // Within each epoch, video starts with a keyframe.
    for epoch in &configs {
        if let Some(first) = videos(&log).into_iter().find(|v| v.1 == *epoch) {
            assert!(first.3, "epoch {epoch} started its video with a keyframe");
        }
    }
    // The tiles-only epoch carried no video.
    let tiles_only: Vec<u32> = log
        .iter()
        .filter_map(|(_, m)| match m {
            Message::Config { epoch, codec: display_protocol::Codec::None, .. } => Some(*epoch),
            _ => None,
        })
        .collect();
    assert_eq!(tiles_only.len(), 1, "one tiles-only epoch: {configs:?}");
    assert!(videos(&log).iter().all(|v| v.1 != tiles_only[0]), "video arrived in the tiles-only epoch");
    viewer.close().await;
    admin.stop(&session).await;
}

#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn auto_prefers_av1_whose_keyframes_carry_the_sequence_header_and_decode() {
    let admin = Admin::login().await;
    let session = admin.create(SCROLL).await;
    let viewer = admin.watch(&session, CODEC_AV1 | CODEC_H264).await;
    wait_video(&viewer, 30).await;
    let log = viewer.log();
    assert_ordered(&log);
    let codec = log.iter().rev().find_map(|(_, m)| match m {
        Message::Config { codec, .. } => Some(*codec),
        _ => None,
    });
    assert_eq!(codec, Some(Codec::Av1), "Auto picks AV1 when the client decodes it");
    let first = log.iter().find_map(|(_, m)| match m {
        Message::Video { keyframe, payload, .. } => Some((*keyframe, payload.clone())),
        _ => None,
    });
    let (keyframe, payload) = first.expect("a video frame");
    assert!(keyframe, "video starts with a keyframe");
    // The first OBU after the temporal delimiter (type 2) is the sequence header (type 1).
    let types: Vec<u8> = [payload[0], payload[2]].iter().map(|h| (h >> 3) & 0x0f).collect();
    assert_eq!(types, [2, 1], "a keyframe opens with a temporal delimiter and the sequence header");
    let video = viewer.look(video_count);
    let frames = decodes(&viewer.video(), "av1", "obu");
    assert!(frames >= video.min(30), "{frames} of {video} AV1 frames decoded");
    viewer.close().await;
    admin.stop(&session).await;
}

/// A session at a size other than ADMIN_OUTPUT's, as the admin page starts one (a common desktop size, 1440x900 for a
/// 16:10 screen): its screen is that size, and its AV1 video decodes. Every other test runs at the default size, which
/// SVT-AV1 never crashed on; one that did (1488x962) crashed the display as the first viewer connected.
#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_session_at_a_common_desktop_size_streams_av1_that_decodes() {
    let admin = Admin::login().await;
    let session = admin.create_with(json!({"url": SCROLL, "width": 1440, "height": 900})).await;
    let viewer = admin.watch(&session, CODEC_AV1).await;
    wait_video(&viewer, 30).await;
    let log = viewer.log();
    let config = log.iter().rev().find_map(|(_, m)| match m {
        Message::Config { width, height, codec, .. } => Some((*width, *height, *codec)),
        _ => None,
    });
    assert_eq!(config, Some((1440, 900, Codec::Av1)));
    let video = videos(&log).len();
    let frames = decodes(&viewer.video(), "av1-desktop", "obu");
    assert!(frames >= video.min(30), "{frames} of {video} AV1 frames decoded");
    viewer.close().await;
    admin.stop(&session).await;
}

/// The picture a viewer shows after its log, as far as its tiles tell: each configuration clears it, each tile message
/// draws its records in order, each moved message copies its moves and then draws its records, and each video frame
/// covers its rectangle with pixels this does not decode (a move carries them with it). RGB rows, with the width and
/// height, and which pixels video covered last.
fn picture(log: &[(Instant, Message)]) -> (usize, usize, Vec<u8>, Vec<bool>) {
    let (mut width, mut height, mut rgb, mut video) = (0usize, 0usize, Vec::new(), Vec::new());
    for (_, message) in log {
        if let Message::Moved { moves, .. } = message {
            for m in moves {
                let (before, covered, to) = (rgb.clone(), video.clone(), m.target());
                for row in 0..usize::from(m.rect.h) {
                    let from = (usize::from(m.rect.y) + row) * width + usize::from(m.rect.x);
                    let at = (usize::from(to.y) + row) * width + usize::from(to.x);
                    let w = usize::from(m.rect.w);
                    rgb[at * 3..(at + w) * 3].copy_from_slice(&before[from * 3..(from + w) * 3]);
                    video[at..at + w].copy_from_slice(&covered[from..from + w]);
                }
            }
        }
        match message {
            Message::Config { width: w, height: h, .. } => {
                (width, height) = (usize::from(*w), usize::from(*h));
                rgb = vec![0; width * height * 3];
                video = vec![false; width * height];
            }
            Message::Tiles { rects, raw_len, data, .. } | Message::Moved { rects, raw_len, data, .. } => {
                if rects.is_empty() {
                    continue;
                }
                for (rect, rows) in rects.iter().zip(common::tiles(rects, *raw_len, data)) {
                    let (x, y, w) = (usize::from(rect.x), usize::from(rect.y), usize::from(rect.w));
                    for (row, line) in rows.chunks(w * 3).enumerate() {
                        let at = (y + row) * width + x;
                        rgb[at * 3..(at + w) * 3].copy_from_slice(line);
                        video[at..at + w].fill(false);
                    }
                }
            }
            Message::Video { rect, .. } => {
                for y in usize::from(rect.y)..usize::from(rect.y + rect.h) {
                    let at = y * width + usize::from(rect.x);
                    video[at..at + usize::from(rect.w)].fill(true);
                }
            }
            _ => {}
        }
    }
    (width, height, rgb, video)
}

/// Waits until `viewer` has received no media for `quiet`.
async fn settle(viewer: &Viewer, quiet: Duration) {
    let settled = |log: &[(Instant, Message)]| {
        let last = log.iter().rev().find(|(_, m)| matches!(m, Message::Tiles { .. }));
        last.is_some_and(|(t, _)| t.elapsed() >= quiet)
    };
    assert!(settled(&log_until(viewer, Duration::from_secs(30), settled).await), "the stream never went quiet");
}

/// What a person sees after moving over a page whose cards change under the pointer (with Chrome's tooltips), and
/// after following a link to another page, is exactly what a viewer that joins afterwards sees: no tile of an earlier
/// picture stays behind.
#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn hovers_and_a_navigation_leave_no_stale_tile() {
    use display_protocol::PointerKind;
    let admin = Admin::login().await;
    let (session, person, held) = admin.hold("https://fixture.bench/hover.html", "Hover").await;
    let at = |x: u16, y: u16| person.pointer(held, PointerKind::Move, x, y);
    // Across every card row and back, pausing on some long enough for their tooltips.
    for round in 0..2u16 {
        for step in 0..60u16 {
            let (x, y) = (20 + step * 21, 140 + (step % 3) * 175 + round * 20);
            at(x, y).await;
            let pause = if step % 20 == 10 { 1500 } else { 40 };
            tokio::time::sleep(Duration::from_millis(pause)).await;
        }
    }
    // Then follows a card's link to the other page, and moves over that one too.
    // (The agent, which could read the title, waits while the person holds control.)
    person.click(held, 200, 180).await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    for step in 0..60u16 {
        at(1260 - step * 21, 160 + (step % 3) * 175).await;
        tokio::time::sleep(Duration::from_millis(if step == 30 { 1500 } else { 40 })).await;
    }
    // The pointer rests where nothing changes under it.
    at(640, 790).await;
    settle(&person, Duration::from_secs(2)).await;
    let joiner = admin.watch(&session, 0).await;
    settle(&joiner, Duration::from_secs(2)).await;
    let (w, h, seen, _) = person.look(picture);
    let (jw, jh, fresh, _) = joiner.look(picture);
    assert_eq!((w, h), (jw, jh));
    let stale: Vec<(usize, usize)> = (0..h.div_ceil(64))
        .flat_map(|ty| (0..w.div_ceil(64)).map(move |tx| (tx, ty)))
        .filter(|&(tx, ty)| {
            (ty * 64..((ty + 1) * 64).min(h)).any(|y| {
                let (from, to) = ((y * w + tx * 64) * 3, (y * w + ((tx + 1) * 64).min(w)) * 3);
                seen[from..to] != fresh[from..to]
            })
        })
        .collect();
    assert!(stale.is_empty(), "{} stale 64-pixel tiles (column, row): {stale:?}", stale.len());
    // The click followed the card's link, so the comparison covered a navigation too.
    admin.give_back(&session, &person).await.expect("give back");
    admin.wait_title(&session, "Hover 2").await;
    joiner.close().await;
    person.close().await;
    admin.stop(&session).await;
}

/// The video frames from the last keyframe on in `log`, decoded (with ffmpeg in the tools image) to RGB pictures of
/// that keyframe's rectangle: the rectangle, how many pictures there are, and the last of them, which alone is read
/// (the pictures of a few seconds take hundreds of MiB).
fn decode_last_video(log: &[(Instant, Message)], name: &str) -> (display_protocol::Rect, usize, Vec<u8>) {
    let start = log.iter().rposition(|(_, m)| matches!(m, Message::Video { keyframe: true, .. })).expect("a keyframe");
    let Message::Video { rect, .. } = log[start].1 else { unreachable!() };
    let stream: Vec<u8> = log[start..]
        .iter()
        .filter_map(|(_, m)| if let Message::Video { payload, .. } = m { Some(payload.as_slice()) } else { None })
        .flatten()
        .copied()
        .collect();
    let dir = common::scratch(name);
    std::fs::write(dir.join("video.obu"), &stream).unwrap();
    let decode = ["ffmpeg", "-v", "error", "-f", "obu", "-i", "/d/video.obu"];
    let out = common::tools(&dir, &[&decode[..], &["-f", "rawvideo", "-pix_fmt", "rgb24", "/d/video.rgb"]].concat());
    assert!(out.status.success(), "ffmpeg: {}", String::from_utf8_lossy(&out.stderr));
    use std::io::{Read, Seek};
    let size = u64::from(rect.w) * u64::from(rect.h) * 3;
    let mut raw = std::fs::File::open(dir.join("video.rgb")).unwrap();
    let pictures = raw.metadata().unwrap().len() / size;
    assert!(pictures > 0, "ffmpeg decoded no picture");
    let mut last = vec![0; size as usize];
    raw.seek(std::io::SeekFrom::Start((pictures - 1) * size)).unwrap();
    raw.read_exact(&mut last).unwrap();
    (rect, pictures as usize, last)
}

/// A clip plays while the page around it changes (a full-screen transition, then other text), or, with `two`, two
/// clips play while a panel between them changes: the person's AV1 viewer's log, the truth at the end (a tiles-only
/// viewer's picture after it settled), and which of its pixels were already so a second before (still ones).
async fn page_change_around_clips(
    query: &str,
) -> (Vec<(Instant, Message)>, (usize, usize, Vec<u8>, Vec<bool>), Vec<bool>) {
    let admin = Admin::login().await;
    let two = query.starts_with("two");
    // At 60 frames a second, which the tests' counts and the transition's cost for each frame are for: at the 30 a
    // session takes otherwise, a frame has twice the bytes to spend, the transition may fit as tiles, and half as
    // many video frames follow the last keyframe.
    let url = format!("https://fixture.bench/region.html?{query}");
    let session = admin.create_with(json!({"url": url, "devtools": false, "max_fps": 60})).await;
    admin.wait_title(&session, "Region").await;
    let person = admin.watch(&session, CODEC_AV1).await;
    let truth = admin.watch(&session, 0).await;
    let held = admin.take(&session, &person).await;
    // With two clips, the video's region holds both before the panel between them changes.
    if two {
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
    person.click(held, 900, 400).await;
    // The transition lasts a second; then only the clips move. What stops after moving for a second settles later.
    tokio::time::sleep(Duration::from_secs(if query.contains("fade") { 6 } else { 4 })).await;
    let before = truth.look(picture).2;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let log = person.log();
    let fresh = truth.look(picture);
    let still = before.chunks(3).zip(fresh.2.chunks(3)).map(|(a, b)| a == b).collect();
    truth.close().await;
    person.close().await;
    admin.stop(&session).await;
    (log, fresh, still)
}

/// A tile of the video's region (column and row) and its mean difference per channel from the truth.
type Tile = (usize, usize, u64);

/// For each 64-pixel tile of the decoded region, the mean difference per channel from the truth (an output-sized RGB
/// picture of `width`) of its pixels that the truth shows still (`steady`, output-sized): in a still part, what remains
/// is AV1's own loss, or traces of an earlier picture.
fn traces(last: &[u8], rect: &display_protocol::Rect, truth: &[u8], steady: &[bool], width: usize) -> Vec<Tile> {
    let (rw, rh) = (usize::from(rect.w), usize::from(rect.h));
    let mut out = Vec::new();
    for ty in 0..rh.div_ceil(64) {
        for tx in 0..rw.div_ceil(64) {
            let (mut sum, mut still) = (0u64, 0u64);
            for y in ty * 64..(ty * 64 + 64).min(rh) {
                for x in tx * 64..(tx * 64 + 64).min(rw) {
                    let at = (y * rw + x) * 3;
                    let truth_at = ((usize::from(rect.y) + y) * width + usize::from(rect.x) + x) * 3;
                    if !steady[truth_at / 3] {
                        continue;
                    }
                    sum += (0..3).map(|c| u64::from(last[at + c].abs_diff(truth[truth_at + c]))).sum::<u64>();
                    still += 3;
                }
            }
            // A tile mostly of a clip's edge says little.
            if still >= 64 * 64 * 3 / 2 {
                out.push((tx, ty, sum / still));
            }
        }
    }
    out
}

/// The largest mean difference per channel a still tile of the video may keep from the truth: AV1's own loss on text,
/// well under what traces of a whole earlier picture leave.
const MAX_TRACE: u64 = 12;

/// region.html's panel between the two clips, in output pixels: x, y, width and height.
const PANEL: (usize, usize, usize, usize) = (440, 200 + common::PAGE_TOP as usize, 400, 300);

/// The still tiles of the video's region off the truth, once every tile of the region wholly inside the panel was
/// compared: a region that shrank, or a panel that never settled, compares nothing there and must not pass.
fn panel_traces(log: &[(Instant, Message)], name: &str, fresh: &[u8], still: &[bool], width: usize) -> Vec<Tile> {
    let (rect, _, last) = decode_last_video(log, name);
    let compared = traces(&last, &rect, fresh, still, width);
    let (px, py, pw, ph) = PANEL;
    let inside = |start: u16, at: usize, from: usize, len: usize| {
        let x = usize::from(start) + at * 64;
        x >= from && x + 64 <= from + len
    };
    let tiles = (0..usize::from(rect.h).div_ceil(64))
        .flat_map(|ty| (0..usize::from(rect.w).div_ceil(64)).map(move |tx| (tx, ty)))
        .filter(|&(tx, ty)| inside(rect.x, tx, px, pw) && inside(rect.y, ty, py, ph));
    let panel: Vec<_> = tiles.collect();
    assert!(!panel.is_empty(), "no tile of the video's region {rect:?} lies inside the panel");
    let missing: Vec<_> = panel.iter().filter(|t| !compared.iter().any(|c| (c.0, c.1) == **t)).collect();
    assert!(missing.is_empty(), "the panel's tiles {missing:?} were not compared (column, row): {compared:?}");
    compared.into_iter().filter(|t| t.2 > MAX_TRACE).collect()
}

/// Two clips in opposite corners keep the page between them inside the video's region: a panel there that changed
/// (in faint colours, which video codes last) keeps no trace of its earlier picture once it stayed still.
#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_page_that_changed_between_two_clips_keeps_no_trace() {
    let (log, (w, _, fresh, _), still) = page_change_around_clips("two").await;
    let off = panel_traces(&log, "region-two", &fresh, &still, w);
    assert!(off.is_empty(), "still tiles of the region off the truth (column, row, difference): {off:?}");
}

/// The same, when the panel fades for a second: motion that then stops is repainted too, after a longer rest.
#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_long_transition_between_two_clips_keeps_no_trace() {
    let (log, (w, _, fresh, _), still) = page_change_around_clips("two&fade").await;
    let off = panel_traces(&log, "region-fade", &fresh, &still, w);
    assert!(off.is_empty(), "still tiles of the region off the truth (column, row, difference): {off:?}");
}

mod exclusive {
    use super::*;

    /// While a clip plays in one corner, the page around it changes (a full-screen transition, then other text): video
    /// then covers the whole screen for a while, but once only the clip moves, it covers the clip alone and goes on, and
    /// everything else is exact again (lossless tiles over whatever video drew there).
    ///
    /// Alone, as the scroll's measure below is. Twice it failed for the rate its session ran at, 30 frames a second
    /// since a session's start picks its rates (too few video frames after the last keyframe, once alone; and, beside
    /// the other suites, a transition that never went as video): its session now asks for the 60 its counts are for.
    /// An earlier release's run failed it beside the other suites too, for a cause not known.
    #[tokio::test]
    #[ignore = "needs the bench stack: scripts/e2e.sh"]
    async fn video_covers_only_where_the_picture_moves() {
        let (log, (w, h, fresh, _), still) = page_change_around_clips("").await;
        let (_, _, seen, video) = picture(&log);
        let rects: Vec<(bool, display_protocol::Rect)> = log
            .iter()
            .filter_map(
                |(_, m)| if let Message::Video { keyframe, rect, .. } = m { Some((*keyframe, *rect)) } else { None },
            )
            .collect();
        let area = |r: &display_protocol::Rect| usize::from(r.w) * usize::from(r.h);
        assert!(rects.iter().any(|(_, r)| area(r) == w * h), "video never covered the whole screen in the transition");
        let last = rects.iter().rposition(|(key, _)| *key).expect("a keyframe");
        let region = rects[last].1;
        // The clip is 360 x 240 (where Chrome's toolbar puts it).
        assert!(area(&region) <= w * h / 5 && region.w >= 360 && region.h >= 240, "the video covers {region:?}");
        let after = rects.len() - last - 1;
        assert!(after >= 30, "only {after} video frames after the last keyframe: the clip stopped going as video");
        let holds = |x: usize, y: usize| {
            (usize::from(region.x)..usize::from(region.x + region.w)).contains(&x)
                && (usize::from(region.y)..usize::from(region.y + region.h)).contains(&y)
        };
        let mut wrong = std::collections::BTreeSet::new();
        for y in 0..h {
            for x in 0..w {
                let at = y * w + x;
                if !holds(x, y) && (video[at] || seen[at * 3..at * 3 + 3] != fresh[at * 3..at * 3 + 3]) {
                    wrong.insert((x / 64, y / 64));
                }
            }
        }
        assert!(
            wrong.is_empty(),
            "{} tiles outside the clip show video or differ (column, row): {wrong:?}",
            wrong.len()
        );
        let (rect, pictures, last) = decode_last_video(&log, "region-one");
        assert_eq!(rect, region);
        assert!(pictures > after / 2, "{pictures} pictures decoded of {}", after + 1);
        let off: Vec<_> = traces(&last, &rect, &fresh, &still, w).into_iter().filter(|t| t.2 > MAX_TRACE).collect();
        assert!(off.is_empty(), "still tiles of the region off the truth (column, row, difference): {off:?}");
    }

    /// A page that scrolls reaches a viewer that takes moves as moves, not video: each frame the picture moved and the
    /// rows it exposed. Once the page stops, that viewer's picture is, pixel for pixel, the one a viewer of lossless
    /// tiles alone has, after hundreds of moves; and it took less than a third of what the video takes.
    ///
    /// Alone: what it compares (the moves' bytes with the video's, over the same five seconds) is for a display that
    /// has its processors. Beside the other suites the scroll now and then went back to video mid-way, and that had a
    /// cause, since removed: a move was judged by how long its encode took by the clock, so on processors shared with other
    /// sessions' displays the scroll left for video (`Encoder::move_fits`; with four busy loops on the display's two
    /// processors, in four runs of four, and in none since). A failure says what the viewer was sent each second, what
    /// came just before the first video frame, and what the display logged of leaving moves.
    #[tokio::test]
    #[ignore = "needs the bench stack: scripts/e2e.sh"]
    async fn a_scrolling_page_goes_as_moves_and_ends_exact() {
        let admin = Admin::login().await;
        // At 60 frames a second, which the count of moves below is for.
        let body = json!({"url": "https://fixture.bench/scroll.html?seconds=12", "devtools": false, "max_fps": 60});
        let session = admin.create_with(body).await;
        let mover = admin.watch(&session, CODEC_AV1 | display_protocol::MOVES).await;
        let video = admin.watch(&session, CODEC_AV1).await;
        let truth = admin.watch(&session, 0).await;
        // The viewers joined mid-scroll: the mover's first frames may be video, until it has the whole picture.
        tokio::time::sleep(Duration::from_secs(4)).await;
        let from = Instant::now();
        tokio::time::sleep(Duration::from_secs(5)).await;
        let until = Instant::now();
        // The page stops at 12 s; what stopped after motion settles a few seconds later.
        tokio::time::sleep(Duration::from_secs(9)).await;
        let (moved_log, video_log) = (mover.log(), video.log());
        fn between(log: &[(Instant, Message)], from: Instant, until: Instant) -> Vec<&Message> {
            log.iter().filter(|(t, _)| *t >= from && *t < until).map(|(_, m)| m).collect()
        }
        let within = |log| between(log, from, until);
        // Whole messages, as they cross the wire: a move's table and a tile's rectangles count.
        let bytes = |messages: &[&Message]| -> usize {
            let media =
                |m: &&&Message| matches!(m, Message::Video { .. } | Message::Tiles { .. } | Message::Moved { .. });
            messages.iter().filter(media).map(|m| m.encode().len()).sum()
        };
        let (moved, filmed) = (within(&moved_log), within(&video_log));
        let moves = moved.iter().filter(|m| matches!(m, Message::Moved { .. })).count();
        let frames = moved.iter().filter(|m| matches!(m, Message::Video { .. })).count();
        // What the mover was sent, second by second since it joined (moves, video frames, tiles messages): a failure says
        // whether it never left video or went back to it, and when. Gathered only for a failure to say.
        let seconds = || {
            let Some((joined, _)) = moved_log.first() else { return String::new() };
            let mut counts = vec![(0, 0, 0); 19];
            for (at, message) in &moved_log {
                let second = (at.duration_since(*joined).as_secs() as usize).min(counts.len() - 1);
                match message {
                    Message::Moved { .. } => counts[second].0 += 1,
                    Message::Video { .. } => counts[second].1 += 1,
                    Message::Tiles { .. } => counts[second].2 += 1,
                    _ => {}
                }
            }
            // And what came just before it first went back to video after a move: each message's kind and bytes,
            // and the milliseconds since the one before.
            let first_move = moved_log.iter().position(|(_, m)| matches!(m, Message::Moved { .. }));
            let back = first_move.and_then(|from| {
                moved_log[from..].iter().position(|(_, m)| matches!(m, Message::Video { .. })).map(|at| from + at)
            });
            let before = back.map_or(String::new(), |at| {
                let said = moved_log[at.saturating_sub(16)..(at + 4).min(moved_log.len())].windows(2).map(|pair| {
                    let gap = pair[1].0.duration_since(pair[0].0).as_millis();
                    let what = match &pair[1].1 {
                        Message::Moved { .. } => "moved".to_owned(),
                        Message::Video { keyframe, .. } => format!("video{}", if *keyframe { " key" } else { "" }),
                        Message::Tiles { .. } => "tiles".to_owned(),
                        other => format!("{other:?}").chars().take(48).collect(),
                    };
                    format!("+{gap} ms {what} {}", pair[1].1.encode().len())
                });
                format!("; before the first video after a move: {}", said.collect::<Vec<_>>().join(", "))
            });
            let (_, logs) =
                common::docker(&["logs", "--tail", "4000", &format!("{}-{session}-display", common::INSTANCE)]);
            // And what its display said of leaving moves, and of taking them up again.
            let said: Vec<&str> =
                logs.lines().filter(|l| l.contains("goes as video") || l.contains("for moves")).collect();
            format!("{counts:?}{before}; the display: {}", said.join(" | "))
        };
        assert_eq!(
            frames,
            0,
            "video frames to the viewer that takes moves, 4 s into the scroll; each second: {}",
            seconds()
        );
        assert!(moves >= 150, "only {moves} moves in 5 s of scrolling; each second: {}", seconds());
        assert!(
            filmed.iter().any(|m| matches!(m, Message::Video { .. })),
            "the other viewer got no video to compare with"
        );
        assert!(
            3 * bytes(&moved) < bytes(&filmed),
            "the moves took {} bytes in 5 s, the video {}",
            bytes(&moved),
            bytes(&filmed)
        );
        let (got, want) = (mover.look(picture), truth.look(picture));
        assert!(!got.3.iter().any(|v| *v), "video still covers a part of the mover's picture");
        let wrong = got.2.chunks(3).zip(want.2.chunks(3)).filter(|(a, b)| a != b).count();
        assert_eq!(wrong, 0, "pixels of the mover's picture differ from the lossless one");
        mover.close().await;
        video.close().await;
        truth.close().await;
        admin.stop(&session).await;
    }
}

/// Sets the session's simulated network: what the admin's relay delays, shapes, and loses on the way to its viewers.
async fn network(admin: &Admin, session: &str, latency_ms: u32, loss_percent: f32, bandwidth_kbps: u32) {
    let body = json!({"latency_ms": latency_ms, "loss_percent": loss_percent, "bandwidth_kbps": bandwidth_kbps});
    admin.call(reqwest::Method::PUT, &format!("/api/sessions/{session}/network"), Some(body)).await.expect("network");
}

/// The bytes of the video and tiles messages in `log` that arrived after `from`.
fn media_bytes_after(log: &[(Instant, Message)], from: Instant) -> usize {
    log.iter()
        .filter(|(t, _)| *t > from)
        .map(|(_, m)| match m {
            Message::Video { payload, .. } => payload.len(),
            Message::Tiles { data, .. } => data.len(),
            _ => 0,
        })
        .sum()
}

/// A viewer whose acknowledgements stop coming gets at most about one window of media, then nothing; once they come
/// again, it gets the current picture at once, not the frames composed meanwhile, and its stream goes on decoding.
#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_viewer_whose_acknowledgements_stop_gets_little_then_current_frames() {
    let admin = Admin::login().await;
    let session = admin.create(SCROLL).await;
    let viewer = admin.watch(&session, CODEC_AV1).await;
    wait_video(&viewer, 30).await;
    viewer.withheld.store(true, std::sync::atomic::Ordering::Relaxed);
    let withheld = Instant::now();
    tokio::time::sleep(Duration::from_secs(4)).await;
    let log = viewer.log();
    let late = log.iter().filter(|(t, m)| *t > withheld + Duration::from_secs(1) && is_media(m)).count();
    assert_eq!(late, 0, "media kept coming a second after the acknowledgements stopped");
    let sent = media_bytes_after(&log, withheld);
    assert!(sent <= 1024 * 1024, "{sent} bytes of media went unacknowledged");
    let before = videos(&log).last().map(|v| v.2);
    let last_shown = log.iter().rev().find_map(|(_, m)| video_time(m)).expect("a video frame");
    viewer.withheld.store(false, std::sync::atomic::Ordering::Relaxed);
    let resumed = Instant::now();
    let log =
        log_until(&viewer, Duration::from_secs(5), |log| videos(log).iter().filter(|v| v.0 > resumed).count() >= 10)
            .await;
    let next = log.iter().filter(|(t, _)| *t > resumed).find_map(|(_, m)| video_time(m)).expect("video resumed");
    // The frame it resumed with was composed after the pause, not the first one composed in it.
    let skipped = Duration::from_micros(next - last_shown);
    assert!(skipped >= Duration::from_secs(3), "it resumed {skipped:?} after the last frame shown");
    assert!(videos(&log).iter().filter(|v| v.0 > resumed).count() >= 10, "video went on after the pause");
    assert!(videos(&log).iter().any(|v| Some(v.2) > before), "newer frames came");
    assert_ordered(&log);
    // Frames were skipped before encoding, so the stream still decodes without a keyframe request.
    decodes(&viewer.video(), "withheld", "obu");
    viewer.close().await;
    admin.stop(&session).await;
}

fn is_media(message: &Message) -> bool {
    matches!(message, Message::Video { .. } | Message::Tiles { .. })
}

fn video_time(message: &Message) -> Option<u64> {
    if let Message::Video { timestamp_us, .. } = message { Some(*timestamp_us) } else { None }
}

/// Behind a slow link (the admin's network simulator: 200 ms of latency and 2 Mbit/s, under what the scrolling page
/// would send), a viewer gets fewer frames, but each about as recent as the link allows: the time from a frame's
/// composition to its arrival does not grow with a backlog.
#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_viewer_behind_a_slow_link_gets_recent_frames_not_a_backlog() {
    let admin = Admin::login().await;
    let session = admin.create(SCROLL).await;
    network(&admin, &session, 200, 0.0, 2000).await;
    let viewer = admin.watch(&session, CODEC_AV1).await;
    wait_video(&viewer, 10).await;
    let start = Instant::now();
    tokio::time::sleep(Duration::from_secs(10)).await;
    let log = viewer.log();
    // Arrival less composition time, up to the offset between the two clocks: its spread is the queueing.
    let delays: Vec<i64> = log
        .iter()
        .filter(|(t, _)| *t > start)
        .filter_map(|(t, m)| {
            let composed = i64::try_from(video_time(m)?).ok()?;
            Some(i64::try_from(t.duration_since(start).as_micros()).ok()? - composed)
        })
        .collect();
    assert!(delays.len() >= 30, "only {} frames in 10 s", delays.len());
    let spread = Duration::from_micros((delays.iter().max().unwrap() - delays.iter().min().unwrap()) as u64);
    assert!(spread < Duration::from_millis(1500), "frames arrived up to {spread:?} later than the most recent one");
    assert_ordered(&log);
    viewer.close().await;
    admin.stop(&session).await;
}

/// Media the simulated network loses is never waited for: a page that changes now and then under heavy loss, its last
/// update lost or not, still shows the next change at once once the loss ends.
#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn lost_media_is_not_waited_for() {
    let admin = Admin::login().await;
    let (session, person, held) = admin.hold("https://fixture.bench/typing.html", "Typing").await;
    settle(&person, Duration::from_millis(500)).await;
    network(&admin, &session, 0, 50.0, 0).await;
    // Each key flips the page's square: one small update, then nothing, half of them lost.
    for _ in 0..16 {
        person.chord(held, &[display_protocol::keys::SPACE]).await;
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
    network(&admin, &session, 0, 0.0, 0).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let before = person.logged();
    person.chord(held, &[display_protocol::keys::SPACE]).await;
    let log = log_until(&person, Duration::from_secs(3), |log| {
        log.iter().skip(before).any(|(_, m)| matches!(m, Message::Tiles { .. }))
    })
    .await;
    assert!(log.iter().skip(before).any(|(_, m)| matches!(m, Message::Tiles { .. })), "the next change never came");
    person.close().await;
    admin.stop(&session).await;
}

/// How many frames a second the page draws: its `requestAnimationFrame` count over `over`, by the page's own clock.
async fn drawn_fps(admin: &Admin, session: &str, (pid, window): (u64, u64), over: Duration) -> f64 {
    let read = || async {
        let script = json!({
            "javascript": "'<' + performance.now().toFixed(0) + ' ' + window.drawn + '>'",
            "pid": pid, "window_id": window,
        });
        let answer = admin.agent(session, "run_javascript", script).await.expect("the count").to_string();
        let inside = answer.split_once('<').and_then(|(_, rest)| rest.split_once('>')).expect("a count").0.to_owned();
        let (at, drawn) = inside.split_once(' ').expect("two numbers");
        (at.parse::<f64>().expect("a time"), drawn.parse::<f64>().expect("a count"))
    };
    let (from, before) = read().await;
    tokio::time::sleep(over).await;
    let (to, after) = read().await;
    (after - before) * 1000.0 / (to - from)
}

/// Makes the page of a session with Developer tools count the frames it draws (`window.drawn`), once it is loaded
/// (a counter set before that would go with the page it was set in): its window.
async fn counting(admin: &Admin, session: &str) -> (u64, u64) {
    admin.wait_title(session, "Inspect").await;
    let page = admin.wait_windows(session, "the page's window", common::first_window).await;
    // A mark that changes with every frame the page is given, so each one is drawn.
    let counting = "window.drawn = 0; const mark = document.createElement('div'); \
        mark.style.cssText = 'position:fixed;left:0;top:0;width:8px;height:8px'; document.body.append(mark); \
        (function frame() { window.drawn++; mark.style.background = window.drawn % 2 ? '#000' : '#fff'; \
        requestAnimationFrame(frame); })(); 'counting'";
    let script = json!({"javascript": counting, "pid": page.0, "window_id": page.1});
    admin.agent(session, "run_javascript", script).await.expect("the counter");
    page
}

/// Waits until the page draws from `low` to `high` frames a second: the screen changes its mode a moment after who
/// watches changes.
async fn settles(admin: &Admin, session: &str, page: (u64, u64), what: &str, low: f64, high: f64) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let fps = drawn_fps(admin, session, page, Duration::from_secs(2)).await;
        if (low..=high).contains(&fps) {
            return;
        }
        assert!(Instant::now() < deadline, "{what}: the page draws {fps:.1} frames a second, not {low}-{high}");
    }
}

/// A session started with CDP that names no rate keeps its screen at the installation's most while nobody watches:
/// what drives it there (Playwright, whose every action waits on the page's frames) works at the pace it always had.
/// With a rate named, it takes that one.
#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_session_with_cdp_keeps_its_screen_s_rate_while_nobody_watches() {
    let admin = Admin::login().await;
    let described = |session: String| {
        let admin = &admin;
        async move {
            let listed = admin.get("/api/sessions").await.expect("the sessions");
            let it = listed["sessions"].as_array().unwrap().iter().find(|s| s["id"] == session.as_str()).expect("it");
            (it["idle_fps"].clone(), it["settings"]["max_fps"].clone())
        }
    };
    let page = "https://fixture.bench/inspect.html";
    let named = admin.create_with(json!({"url": page, "cdp": true, "idle_fps": 5, "max_fps": 30})).await;
    assert_eq!(described(named).await, (json!(5), json!(30)));
    let session = admin.create_with(json!({"url": page, "cdp": true})).await;
    assert_eq!(described(session.clone()).await, (json!(60), json!(60)));
    let window = counting(&admin, &session).await;
    settles(&admin, &session, window, "nobody watches a session with CDP", 50.0, 64.0).await;
    admin.stop(&session).await;
}

/// A session's screen refreshes at its idle rate while nobody watches, at what its viewer takes while one does, and
/// at its idle rate again once they leave: the page, which draws no faster than its screen, shows each.
#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn the_screen_refreshes_at_the_idle_rate_until_someone_watches() {
    let admin = Admin::login().await;
    let body = json!({"url": "https://fixture.bench/inspect.html", "devtools": true, "idle_fps": 5, "max_fps": 30});
    let session = admin.create_with(body).await;
    let listed = admin.get("/api/sessions").await.expect("the sessions");
    let described = listed["sessions"].as_array().unwrap().iter().find(|s| s["id"] == session.as_str()).expect("it");
    assert_eq!((&described["idle_fps"], &described["settings"]["max_fps"]), (&json!(5), &json!(30)), "{described}");
    let page = counting(&admin, &session).await;
    let settles = |what: &'static str, low: f64, high: f64| settles(&admin, &session, page, what, low, high);
    settles("nobody watches", 3.0, 7.0).await;
    let viewer = admin.watch(&session, CODEC_AV1).await;
    settles("a viewer at 30", 20.0, 34.0).await;
    // What the viewer takes may change while the session runs, up to the installation's maximum and no further.
    let path = format!("/api/sessions/{session}/settings");
    let at = |fps: u32| Some(json!({"codec": "auto", "bitrate_kbps": 2500, "max_fps": fps}));
    admin.call(reqwest::Method::PUT, &path, at(60)).await.expect("60 frames a second");
    settles("a viewer at 60", 38.0, 64.0).await;
    let refused = admin.call(reqwest::Method::PUT, &path, at(61)).await.expect_err("past the maximum");
    assert_eq!(refused.0, 400, "{}", refused.1);
    viewer.close().await;
    settles("the viewer left", 3.0, 7.0).await;
    // A start outside what the installation takes is refused.
    for rates in [json!({"max_fps": 61}), json!({"idle_fps": 4}), json!({"idle_fps": 31})] {
        let mut body = json!({"url": "https://fixture.bench/inspect.html"});
        body.as_object_mut().unwrap().extend(rates.as_object().unwrap().clone());
        let refused = admin.call(reqwest::Method::POST, "/api/sessions", Some(body)).await.expect_err("refused");
        assert_eq!(refused.0, 400, "{rates}: {}", refused.1);
    }
    admin.stop(&session).await;
}
