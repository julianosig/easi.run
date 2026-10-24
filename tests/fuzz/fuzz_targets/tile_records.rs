//! Tile decoding as the end-to-end tests do it: any server message that decodes as `Tiles` either inflates to exactly
//! its declared records length, without reading past it, and decodes to one RGB tile per rectangle, or is refused.
#![no_main]
use std::io::Read;

use display_protocol::{Direction, Message, records};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(Message::Tiles { rects, raw_len, data, .. }) = Message::decode(data, Direction::ServerToClient) else {
        return;
    };
    let mut raw = Vec::new();
    let mut inflate = flate2::read::DeflateDecoder::new(&data[..]).take(u64::from(raw_len) + 1);
    if inflate.read_to_end(&mut raw).is_err() || raw.len() != raw_len as usize {
        return;
    }
    if let Ok(tiles) = records::decode(&rects, &raw) {
        assert_eq!(tiles.len(), rects.len());
        for (rect, rgb) in rects.iter().zip(tiles) {
            assert_eq!(rgb.len(), 3 * usize::from(rect.w) * usize::from(rect.h));
        }
    }
});
