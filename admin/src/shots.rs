//! The screenshots a tool answers with, smaller. The driver makes each a 32-bit PNG that it barely compresses (about
//! 300 kB for a page at 1280x800), and the programs that drive a session keep every one in their conversation for
//! good: Codex and Claude Code write each tool answer whole into the conversation's file, which was 11 of the 14.5 GB
//! of one conversation of eleven days. Here the same picture is made a PNG of 256 colours, about a quarter of the
//! size: still a PNG of the same size in pixels, so whatever reads the driver's reads this, and a point in it is the
//! same point.
//!
//! That loses colour: 256 are chosen for each picture, so two that are close may become one. Text reads the same (64
//! codes of 64, at 10 to 16 pixels and several contrasts, by both programs' models, as from the driver's own); what a
//! colour is exactly, an agent reads from the page (`inspect_page`), never from a picture.
//!
//! Only a picture as the driver makes one is touched: an image item of an answer's own content that is a still PNG of
//! eight bits a channel, in colour, with nothing that is see-through and nothing that says how its colours are to be
//! read. What a tool shapes itself elsewhere in its answer (a script's value) is never taken for one.
//! Anything else, and a picture that would not come out smaller, is answered as it came. One picture at a time for
//! the whole admin, off the threads that serve everyone else: what one takes in memory is bounded by its size, and
//! none waits long for its turn.

use std::time::Duration;

use base64::Engine;
use serde_json::Value;

/// What decoding one may take beyond its pixels: the decoder's own.
const MAX_DECODER: usize = 4 * 1024 * 1024;
/// The pictures of one answer that are made smaller: a `run_steps` with more leaves the rest as they came.
const MAX_PICTURES: usize = 16;
/// How long a picture waits for its turn before it is answered as it came.
const TURN_WITHIN: Duration = Duration::from_secs(2);
/// One at a time (see the module).
static TURN: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);

const SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

/// The size a PNG's first 33 bytes say, if they are those of a picture this takes: eight bits a channel, in colour
/// with or without alpha, not interlaced, and no larger than a display is.
fn headed(header: &[u8]) -> Option<(u32, u32)> {
    let header = header.get(..33)?;
    if &header[..8] != SIGNATURE || header[8..16] != [0, 0, 0, 13, b'I', b'H', b'D', b'R'] {
        return None;
    }
    let size = |at: usize| u32::from_be_bytes([header[at], header[at + 1], header[at + 2], header[at + 3]]);
    let (width, height) = (size(16), size(20));
    let (depth, colour, interlaced) = (header[24], header[25], header[28]);
    let displayed = (1..=*display_protocol::frame::OUTPUT_WIDTH.end()).contains(&width)
        && (1..=*display_protocol::frame::OUTPUT_HEIGHT.end()).contains(&height);
    (depth == 8 && matches!(colour, 2 | 6) && interlaced == 0 && displayed).then_some((width, height))
}

/// The size of `png`, if it is a picture this takes, told before anything of it is decoded: what its header says
/// (see [`headed`]), and made of nothing but its header, its pixels and its end. Whatever else a PNG may carry (a
/// palette, frames, what is see-through, how its colours are to be read or its pixels shaped) makes it another's
/// picture, left as it is: none of that would be in what is made of it.
fn taken(png: &[u8]) -> Option<(u32, u32)> {
    let size = headed(png)?;
    // Chunk by chunk to its end, which is the file's.
    let mut at = 8usize;
    loop {
        let chunk = png.get(at..at.checked_add(8)?)?;
        let length = u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]) as usize;
        let next = at.checked_add(12)?.checked_add(length)?;
        if next > png.len() {
            return None;
        }
        match &chunk[4..8] {
            b"IEND" => return (next == png.len()).then_some(size),
            b"IHDR" | b"IDAT" => at = next,
            _ => return None,
        }
    }
}

/// 256 colours for `pixels` (`step` bytes each, red, green and blue first) by median cut, and each pixel's: the
/// colours are counted in a histogram of 15 bits, a box of them is cut in two at the median of its pixels along its
/// longest side, the most populous spread-out box first, and a box's colour is the mean of its pixels.
fn quantized(pixels: &[u8], step: usize) -> (Vec<u8>, Vec<u8>) {
    const BINS: usize = 1 << 15;
    let bin = |p: &[u8]| ((p[0] as usize >> 3) << 10) | ((p[1] as usize >> 3) << 5) | (p[2] as usize >> 3);
    let side_of = |bin: usize, side: usize| ((bin >> (10 - 5 * side)) & 31) as u32;
    let mut count = vec![0u32; BINS];
    let mut sums = vec![[0u64; 3]; BINS];
    for p in pixels.chunks_exact(step) {
        let at = bin(p);
        count[at] += 1;
        for side in 0..3 {
            sums[at][side] += u64::from(p[side]);
        }
    }
    struct Cut {
        bins: Vec<u16>,
        pixels: u64,
        side: usize,
        spread: u32,
    }
    let measured = |bins: Vec<u16>| {
        let (mut least, mut most, mut pixels) = ([31u32; 3], [0u32; 3], 0u64);
        for &at in &bins {
            pixels += u64::from(count[usize::from(at)]);
            for side in 0..3 {
                let value = side_of(usize::from(at), side);
                least[side] = least[side].min(value);
                most[side] = most[side].max(value);
            }
        }
        let side = (0..3).max_by_key(|&side| most[side] - least[side]).unwrap_or(0);
        Cut { bins, pixels, side, spread: most[side] - least[side] }
    };
    let used: Vec<u16> = (0..BINS).filter(|&at| count[at] > 0).map(|at| at as u16).collect();
    let mut cuts = vec![measured(used)];
    while cuts.len() < 256 {
        let next = (0..cuts.len())
            .filter(|&i| cuts[i].bins.len() > 1)
            .max_by_key(|&i| cuts[i].pixels * (u64::from(cuts[i].spread) + 1));
        let Some(next) = next else { break };
        let mut cut = cuts.swap_remove(next);
        let side = cut.side;
        cut.bins.sort_unstable_by_key(|&at| side_of(usize::from(at), side));
        let (mut seen, mut split) = (0u64, 1);
        for (i, &at) in cut.bins.iter().enumerate() {
            seen += u64::from(count[usize::from(at)]);
            if seen * 2 >= cut.pixels {
                split = (i + 1).clamp(1, cut.bins.len() - 1);
                break;
            }
        }
        let upper = cut.bins.split_off(split);
        cuts.push(measured(cut.bins));
        cuts.push(measured(upper));
    }
    let mut palette = Vec::with_capacity(cuts.len() * 3);
    let mut index = vec![0u8; BINS];
    for (i, cut) in cuts.iter().enumerate() {
        let mut sum = [0u64; 3];
        for &at in &cut.bins {
            index[usize::from(at)] = i as u8;
            for side in 0..3 {
                sum[side] += sums[usize::from(at)][side];
            }
        }
        palette.extend(sum.map(|s| (s / cut.pixels.max(1)) as u8));
    }
    (palette, pixels.chunks_exact(step).map(|p| index[bin(p)]).collect())
}

/// What making a picture of `width` by `height` smaller holds beyond the picture itself: its pixels as decoded (four
/// bytes each at most), each one's colour, the decoder's own, and what comes of it (less than its pixels' colours).
fn scratch(width: u32, height: u32) -> usize {
    MAX_DECODER + 6 * (width as usize) * (height as usize)
}

/// `png` in 256 colours, if it is a picture this takes (see the module).
pub fn slim(png: &[u8]) -> Option<Vec<u8>> {
    let (width, height) = taken(png)?;
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png));
    decoder.set_limits(png::Limits { bytes: MAX_DECODER });
    decoder.set_transformations(png::Transformations::IDENTITY);
    // A picture whose chunks do not add up is none: their checks are made.
    decoder.ignore_checksums(false);
    let mut reader = decoder.read_info().ok()?;
    if (reader.info().width, reader.info().height) != (width, height) {
        return None;
    }
    let mut pixels = vec![0; reader.output_buffer_size()?];
    let frame = reader.next_frame(&mut pixels).ok()?;
    pixels.truncate(frame.buffer_size());
    let step = match (frame.color_type, frame.bit_depth) {
        (png::ColorType::Rgb, png::BitDepth::Eight) => 3,
        (png::ColorType::Rgba, png::BitDepth::Eight) => 4,
        _ => return None,
    };
    // All of it, to its end: a picture cut short is none.
    reader.finish().ok()?;
    if pixels.len() != step * (width as usize) * (height as usize) {
        return None;
    }
    // Nothing see-through.
    if step == 4 && pixels.as_chunks::<4>().0.iter().any(|pixel| pixel[3] != 255) {
        return None;
    }
    let (palette, indices) = quantized(&pixels, step);
    drop(pixels);
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Indexed);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_palette(palette);
    encoder.set_compression(png::Compression::High);
    encoder.write_header().ok()?.write_image_data(&indices).ok()?;
    Some(out)
}

/// The pictures of `value` that may be made smaller: the data of each image item of the answer's own content that
/// says it is a PNG, which is where a tool's screenshot is (a `run_steps`'s too). What an answer holds elsewhere is
/// its tool's to shape, a script's own value for one, and is never taken for a screenshot.
fn pictures<'a>(value: &'a mut Value, found: &mut Vec<&'a mut String>) {
    let Some(items) = value.get_mut("content").and_then(Value::as_array_mut) else { return };
    for item in items {
        let Value::Object(fields) = item else { continue };
        let image = fields.get("type").and_then(Value::as_str) == Some("image")
            && fields.get("mimeType").and_then(Value::as_str) == Some("image/png");
        if image && let Some(Value::String(data)) = fields.get_mut("data") {
            found.push(data);
        }
    }
}

/// The size the picture in `data` (base64) says it has, from its first bytes alone, if it is one this takes.
fn peeked(data: &str) -> Option<(u32, u32)> {
    let mut header = [0u8; 33];
    let start = data.as_bytes().get(..44)?;
    let read = base64::engine::general_purpose::STANDARD.decode_slice(start, &mut header).ok()?;
    headed(&header[..read])
}

/// `data` (a picture in base64 that says it is `size`) made smaller, or as it came: what this is given back whatever
/// happens to the work.
fn smaller(data: String, size: (u32, u32)) -> String {
    let engine = base64::engine::general_purpose::STANDARD;
    let made = std::panic::catch_unwind(|| {
        let png = engine.decode(data.as_bytes()).ok()?;
        // As large as it said when room was made for it, and no larger.
        (taken(&png)? == size).then_some(())?;
        let slim = engine.encode(slim(&png)?);
        (slim.len() < data.len()).then_some(slim)
    });
    made.ok().flatten().unwrap_or(data)
}

/// `value` with its pictures made smaller (see the module). What the work on one holds (the picture in and out, both
/// ways it is written, and [`scratch`]) is taken from the memory the agents' answers share once it is the picture's
/// turn and before it leaves the answer, never waited for, and kept until it is the answer's again or the work ends,
/// whatever becomes of who asked: a picture there is no room for is answered as it came.
pub async fn slimmed(mut value: Value) -> Value {
    let mut found = Vec::new();
    pictures(&mut value, &mut found);
    for data in found.into_iter().take(MAX_PICTURES) {
        let Some(size) = peeked(data) else { continue };
        // Its turn, or as it came: the pictures of many agents at once are made one after the other, and only the one
        // whose turn it is holds room for the work.
        let Ok(Ok(turn)) = tokio::time::timeout(TURN_WITHIN, TURN.acquire()).await else { break };
        let Some(room) = crate::agent::scratch(2 * data.len() + scratch(size.0, size.1)) else { continue };
        let given = std::mem::take(data);
        let made = tokio::task::spawn_blocking(move || {
            let made = smaller(given, size);
            drop(turn);
            (made, room)
        });
        // The work gives back what it was given whatever happens in it (only an admin that is stopping has none), with
        // its room, kept until the picture is the answer's again.
        if let Ok((made, room)) = made.await {
            *data = made;
            drop(room);
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn encoded(width: u32, height: u32, colour: png::ColorType, pixels: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(colour);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        encoder.write_header().unwrap().write_image_data(pixels).unwrap();
        out
    }

    /// A page as the driver pictures one: 32 bits a pixel, flat areas, edges of text in many shades.
    fn page(width: u32, height: u32) -> Vec<u8> {
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                let ink = (x / 3 + y / 7) % 11 == 0 && y % 20 < 12;
                let shade = if ink { 30 + ((x * 7 + y * 3) % 60) as u8 } else { 250 };
                pixels.extend_from_slice(&[shade, shade, shade.saturating_add(5), 255]);
            }
        }
        encoded(width, height, png::ColorType::Rgba, &pixels)
    }

    fn decoded(png: &[u8]) -> (png::OutputInfo, Vec<u8>) {
        let mut decoder = png::Decoder::new(std::io::Cursor::new(png));
        decoder.set_transformations(png::Transformations::EXPAND);
        let mut reader = decoder.read_info().unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        let frame = reader.next_frame(&mut pixels).unwrap();
        reader.finish().unwrap();
        pixels.truncate(frame.buffer_size());
        (frame, pixels)
    }

    #[test]
    fn a_screenshot_is_the_same_picture_in_256_colours_and_smaller() {
        let shot = page(640, 400);
        let smaller = slim(&shot).expect("a page is made smaller");
        assert!(smaller.len() * 2 < shot.len(), "{} of {}", smaller.len(), shot.len());
        // Still a PNG of the same size, its colours now a palette, and every pixel near what it was: a page of few
        // colours keeps them, to the step of the histogram.
        assert_eq!(&smaller[..8], SIGNATURE);
        assert_eq!(smaller[25], 3, "an indexed PNG");
        let ((was, before), (is, after)) = (decoded(&shot), decoded(&smaller));
        assert_eq!((is.width, is.height), (was.width, was.height));
        let (before, after) = (before.as_chunks::<4>().0, after.as_chunks::<3>().0);
        let far = before.iter().zip(after).filter(|(a, b)| (0..3).any(|c| a[c].abs_diff(b[c]) > 8));
        assert_eq!(far.count(), 0, "no pixel is far from what it was");
        // A picture of two colours keeps exactly those.
        let two: Vec<u8> = (0..64 * 64).flat_map(|i| if i % 3 == 0 { [200, 10, 10] } else { [10, 10, 200] }).collect();
        let (_, kept) = decoded(&slim(&encoded(64, 64, png::ColorType::Rgb, &two)).unwrap());
        assert_eq!(kept, two);
    }

    #[test]
    fn what_is_no_picture_as_the_driver_makes_one_is_left_as_it_came() {
        let shot = page(64, 40);
        assert_eq!(slim(b"not a picture"), None);
        assert_eq!(slim(&shot[..shot.len() / 2]), None, "a picture cut short");
        assert_eq!(slim(&shot[..shot.len() - 12]), None, "a picture without its end");
        // See-through anywhere, grey, or a palette of its own: not the driver's, and what they mean is kept.
        let mut clear = decoded(&shot).1;
        clear[3] = 0;
        assert_eq!(slim(&encoded(64, 40, png::ColorType::Rgba, &clear)), None);
        assert_eq!(slim(&encoded(8, 8, png::ColorType::Grayscale, &[7; 64])), None);
        assert_eq!(slim(&slim(&page(640, 400)).unwrap()), None, "already in 256 colours");
        // A size no display has is refused by its header alone, either way.
        for at in [16, 20] {
            let mut huge = shot.clone();
            huge[at..at + 4].copy_from_slice(&4_000u32.to_be_bytes());
            assert_eq!(taken(&huge), None);
        }
        assert_eq!(taken(&shot), Some((64, 40)));
        // Anything in it besides its header, its pixels and its end makes it another's: here, how its colours are to
        // be read (a gAMA chunk before its pixels), which what is made of it would not say.
        let mut told = shot[..33].to_vec();
        told.extend_from_slice(&[0, 0, 0, 4, b'g', b'A', b'M', b'A', 0, 0, 0xb1, 0x8f, 0x0b, 0xfc, 0x61, 0x05]);
        told.extend_from_slice(&shot[33..]);
        assert_eq!(taken(&told), None);
        // One whose bytes do not add up is none: a byte of its pixels changed after it was made.
        let mut wrong = shot.clone();
        let middle = wrong.len() / 2;
        wrong[middle] ^= 0x55;
        assert_eq!(slim(&wrong), None);
    }

    #[tokio::test]
    async fn only_the_screenshots_of_an_answer_s_own_content_change() {
        let engine = base64::engine::general_purpose::STANDARD;
        let shot = engine.encode(page(320, 200));
        let answer = json!({
            "content": [
                {"type": "image", "data": shot, "mimeType": "image/png"},
                {"type": "text", "text": shot},
                {"type": "image", "data": shot, "mimeType": "image/jpeg"},
                {"type": "image", "data": "not base64 !", "mimeType": "image/png"},
                {"type": "image", "data": shot, "mimeType": "image/png"},
            ],
            // What a script answered, shaped as it pleased: its own, whatever it looks like.
            "structuredContent": {"value": {"type": "image", "data": shot, "mimeType": "image/png",
                "content": [{"type": "image", "data": shot, "mimeType": "image/png"}]}},
        });
        let got = slimmed(answer.clone()).await;
        let kind = |value: &Value| engine.decode(value["data"].as_str().unwrap()).unwrap()[25];
        assert_eq!((kind(&got["content"][0]), kind(&got["content"][4])), (3, 3));
        assert!(got["content"][0]["data"].as_str().unwrap().len() < shot.len());
        // A text that is a picture's bytes, a picture of another kind, and one that is none stay as they came.
        for at in 1..4 {
            assert_eq!(got["content"][at], answer["content"][at], "{at}");
        }
        assert_eq!(got["structuredContent"], answer["structuredContent"]);
        // An answer with no picture, or with no content at all, is the same answer.
        for plain in [json!({"content": [{"type": "text", "text": "ok"}]}), json!({"ok": true}), json!("text")] {
            assert_eq!(slimmed(plain.clone()).await, plain);
        }
    }
}
