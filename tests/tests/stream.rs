//! The ordered hybrid stream's recovery (docs/design.md §3.3), through the running admin application and real Chromium
//! frames. The video checks decode with ffmpeg in the `easi-run/tools:dev` image (bench/tools).

mod common;

use std::time::{Duration, Instant};

use common::{Admin, Viewer};
use display_protocol::{CODEC_AV1, CODEC_H264, Codec, Message};
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

/// The tiles of the lossless refresh that follows the last video frame in `log`: the first revision after that frame's,
/// possibly split over several messages.
fn final_refresh(log: &[(Instant, Message)]) -> Option<usize> {
    let last_video = videos(log).last()?.2;
    let tiles = |m: &Message| match m {
        Message::Tiles { revision, rects, .. } if *revision > last_video => Some((*revision, rects.len())),
        _ => None,
    };
    let refresh = log.iter().filter_map(|(_, m)| tiles(m)).map(|t| t.0).min()?;
    Some(log.iter().filter_map(|(_, m)| tiles(m)).filter(|t| t.0 == refresh).map(|t| t.1).sum())
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
            Message::Video { epoch: e, revision: r, .. } | Message::Tiles { epoch: e, revision: r, .. } => {
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
        let log = viewer.log();
        if done(&log) || Instant::now() >= deadline {
            return log;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn wait_video(viewer: &Viewer, at_least: usize) {
    let log = log_until(viewer, Duration::from_secs(15), |log| videos(log).len() >= at_least).await;
    assert!(videos(&log).len() >= at_least, "no video arrived (is the page moving?)");
}

#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn video_starts_with_a_keyframe_and_ends_with_a_newer_lossless_refresh() {
    let admin = Admin::login().await;
    let session = admin.create(&format!("{SCROLL}?seconds=6")).await;
    let viewer = admin.watch(&session, CODEC_AV1).await;
    // The six seconds of scrolling start as the page loads; then the stream leaves video.
    admin.wait_title(&session, "Smooth scroll").await;
    let log = log_until(&viewer, Duration::from_secs(30), |log| {
        let quiet = videos(log).last().is_some_and(|v| v.0.elapsed() >= Duration::from_secs(1));
        quiet && final_refresh(log).is_some_and(|tiles| tiles == 20 * 13)
    })
    .await;
    assert_ordered(&log);
    let video = videos(&log);
    assert!(video.len() >= 30, "scrolling became video ({} frames)", video.len());
    assert!(video[0].3, "video starts with a keyframe");
    let refresh = final_refresh(&log).expect("leaving video ends with lossless tiles at a newer revision");
    assert_eq!(refresh, 20 * 13, "the final refresh covers the whole screen");
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
    let before = videos(&fast.log()).len();
    let start = Instant::now();
    tokio::time::sleep(Duration::from_secs(3)).await;
    let fast_rate = (videos(&fast.log()).len() - before) as f64 / start.elapsed().as_secs_f64();

    slow.paused.store(true, std::sync::atomic::Ordering::Relaxed);
    let paused_at = Instant::now();
    let fast_before = videos(&fast.log()).len();
    tokio::time::sleep(Duration::from_secs(12)).await;
    let fast_while = (videos(&fast.log()).len() - fast_before) as f64 / paused_at.elapsed().as_secs_f64();
    slow.paused.store(false, std::sync::atomic::Ordering::Relaxed);
    let slow_before = videos(&slow.log()).len();
    tokio::time::sleep(Duration::from_secs(5)).await;
    let slow_after = videos(&slow.log()).len() - slow_before;

    assert!(
        fast_while >= fast_rate * 0.7,
        "the fast viewer kept its rate: {fast_rate:.1} fps before, {fast_while:.1} fps while the other was stalled"
    );
    assert!(slow_after >= 10, "the slow viewer recovered ({slow_after} frames after resuming)");
    assert_ordered(&slow.log());
    assert_ordered(&fast.log());
    // Frames were skipped before encoding, so the slow viewer's stream still decodes without a keyframe request.
    let decoded = decodes(&slow.video(), "slow", "obu");
    assert_eq!(decoded, videos(&slow.log()).len());
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
    let video = videos(&viewer.log()).len();
    let frames = decodes(&viewer.video(), "av1", "obu");
    assert!(frames >= video.min(30), "{frames} of {video} AV1 frames decoded");
    viewer.close().await;
    admin.stop(&session).await;
}

/// The picture a viewer shows after its log, as far as its tiles tell: each configuration clears it, each tile message
/// draws its records in order, and each video frame covers its rectangle with pixels this does not decode. RGB rows,
/// with the width and height, and which pixels video covered last.
fn picture(log: &[(Instant, Message)]) -> (usize, usize, Vec<u8>, Vec<bool>) {
    let (mut width, mut height, mut rgb, mut video) = (0usize, 0usize, Vec::new(), Vec::new());
    for (_, message) in log {
        match message {
            Message::Config { width: w, height: h, .. } => {
                (width, height) = (usize::from(*w), usize::from(*h));
                rgb = vec![0; width * height * 3];
                video = vec![false; width * height];
            }
            Message::Tiles { rects, raw_len, data, .. } => {
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
    let (w, h, seen, _) = picture(&person.log());
    let (jw, jh, fresh, _) = picture(&joiner.log());
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
/// that keyframe's rectangle, with the rectangle.
fn decode_last_video(log: &[(Instant, Message)], name: &str) -> (display_protocol::Rect, Vec<Vec<u8>>) {
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
    let raw = std::fs::read(dir.join("video.rgb")).unwrap();
    let size = usize::from(rect.w) * usize::from(rect.h) * 3;
    (rect, raw.chunks_exact(size).map(<[u8]>::to_vec).collect())
}

/// A clip plays while the page around it changes (a full-screen transition, then other text), or, with `two`, two
/// clips play while a panel between them changes: the person's AV1 viewer's log, the truth at the end (a tiles-only
/// viewer's picture after it settled), and which of its pixels were already so a second before (still ones).
async fn page_change_around_clips(
    query: &str,
) -> (Vec<(Instant, Message)>, (usize, usize, Vec<u8>, Vec<bool>), Vec<bool>) {
    let admin = Admin::login().await;
    let two = query.starts_with("two");
    let session = admin.create(&format!("https://fixture.bench/region.html?{query}")).await;
    let person = admin.watch(&session, CODEC_AV1).await;
    let truth = admin.watch(&session, 0).await;
    admin.wait_title(&session, "Region").await;
    let held = admin.take(&session, &person).await;
    // With two clips, the video's region holds both before the panel between them changes.
    if two {
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
    person.click(held, 900, 400).await;
    // The transition lasts a second; then only the clips move. What stops after moving for a second settles later.
    tokio::time::sleep(Duration::from_secs(if query.contains("fade") { 6 } else { 4 })).await;
    let before = picture(&truth.log()).2;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let log = person.log();
    let fresh = picture(&truth.log());
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
fn traces(
    pictures: &[Vec<u8>],
    rect: &display_protocol::Rect,
    truth: &[u8],
    steady: &[bool],
    width: usize,
) -> Vec<Tile> {
    let last = pictures.last().unwrap();
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

/// While a clip plays in one corner, the page around it changes (a full-screen transition, then other text): video
/// then covers the whole screen for a while, but once only the clip moves, it covers the clip alone and goes on, and
/// everything else is exact again (lossless tiles over whatever video drew there).
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
    assert!(wrong.is_empty(), "{} tiles outside the clip show video or differ (column, row): {wrong:?}", wrong.len());
    let (rect, pictures) = decode_last_video(&log, "region-one");
    assert_eq!(rect, region);
    assert!(pictures.len() > after / 2, "{} pictures decoded of {}", pictures.len(), after + 1);
    let off: Vec<_> = traces(&pictures, &rect, &fresh, &still, w).into_iter().filter(|t| t.2 > MAX_TRACE).collect();
    assert!(off.is_empty(), "still tiles of the region off the truth (column, row, difference): {off:?}");
}

/// The largest mean difference per channel a still tile of the video may keep from the truth: AV1's own loss on text,
/// well under what traces of a whole earlier picture leave.
const MAX_TRACE: u64 = 12;

/// region.html's panel between the two clips, in output pixels: x, y, width and height.
const PANEL: (usize, usize, usize, usize) = (440, 200 + common::PAGE_TOP as usize, 400, 300);

/// The still tiles of the video's region off the truth, once every tile of the region wholly inside the panel was
/// compared: a region that shrank, or a panel that never settled, compares nothing there and must not pass.
fn panel_traces(log: &[(Instant, Message)], name: &str, fresh: &[u8], still: &[bool], width: usize) -> Vec<Tile> {
    let (rect, pictures) = decode_last_video(log, name);
    let compared = traces(&pictures, &rect, fresh, still, width);
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
