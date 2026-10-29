//! A `Tiles` message's records as a viewer decodes them, shared by the end-to-end helpers and the `tile_records` fuzz
//! target (which includes this file).

use std::io::Read;

/// The RGB pixels of each rectangle of a `Tiles` message, or `None` when its records do not inflate, reading at most one
/// byte past the declared length (so a malformed stream cannot exhaust memory), to exactly that length, or then do not
/// decode.
pub fn decode(rects: &[display_protocol::Rect], raw_len: u32, data: &[u8]) -> Option<Vec<Vec<u8>>> {
    let mut raw = Vec::new();
    let mut inflate = flate2::read::DeflateDecoder::new(data).take(u64::from(raw_len) + 1);
    inflate.read_to_end(&mut raw).ok().filter(|&n| n == raw_len as usize)?;
    display_protocol::records::decode(rects, &raw).ok()
}
