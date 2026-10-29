//! Tile decoding as the end-to-end tests do it (`tests/tests/common/tile_records.rs`, included): any server message that
//! decodes as `Tiles` either inflates to exactly its declared records length, without reading past it, and decodes to
//! one RGB tile per rectangle, or is refused.
#![no_main]
use display_protocol::{Direction, Message};
use libfuzzer_sys::fuzz_target;

#[path = "../../tests/common/tile_records.rs"]
mod tile_records;

fuzz_target!(|data: &[u8]| {
    let Ok(Message::Tiles { rects, raw_len, data, .. }) = Message::decode(data, Direction::ServerToClient) else {
        return;
    };
    if let Some(tiles) = tile_records::decode(&rects, raw_len, &data) {
        assert_eq!(tiles.len(), rects.len());
        for (rect, rgb) in rects.iter().zip(tiles) {
            assert_eq!(rgb.len(), 3 * usize::from(rect.w) * usize::from(rect.h));
        }
    }
});
