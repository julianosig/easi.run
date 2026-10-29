//! Tile records: the inflated content of a [`Message::Tiles`](crate::Message::Tiles), one record per rectangle in
//! order. A record is a one-byte tag and its body:
//!
//! - [`RGB`]: the rectangle's 8-bit RGB rows, top row first (`w * h * 3` bytes);
//! - [`SOLID`]: one RGB color that fills the rectangle (3 bytes);
//! - [`PALETTE`]: the color count minus one (`n - 1`, `n` from 2 to 256), `n` RGB colors, then one index byte per
//!   pixel, row by row, each below `n`. A palette record is never larger than the same rectangle in RGB.
//!
//! The records' total length is the message's `raw_len`, which [`bounds`] keeps within what the rectangles allow.

use crate::{Error, Reader, Rect};

pub const RGB: u8 = 0;
pub const SOLID: u8 = 1;
pub const PALETTE: u8 = 2;

/// The shortest and the longest records of these rectangles: every one solid, and every one RGB.
pub fn bounds(rects: &[Rect]) -> (usize, usize) {
    let longest = rects.iter().map(|r| 1 + 3 * usize::from(r.w) * usize::from(r.h)).sum();
    (4 * rects.len(), longest)
}

/// Whether a palette of `n` colors is no larger than RGB for a rectangle of `pixels`.
pub fn palette_fits(n: usize, pixels: usize) -> bool {
    (2..=256).contains(&n) && 1 + 3 * n + pixels <= 3 * pixels
}

/// Decodes the records of `rects` from `raw` into each rectangle's RGB rows. Fails on an unknown tag, a short or
/// over-long body, an index out of its palette, or bytes left over.
pub fn decode(rects: &[Rect], raw: &[u8]) -> Result<Vec<Vec<u8>>, Error> {
    let mut r = Reader(raw);
    let mut out = Vec::with_capacity(rects.len());
    for rect in rects {
        let pixels = usize::from(rect.w) * usize::from(rect.h);
        let rgb = match r.u8()? {
            RGB => r.take(3 * pixels)?.to_vec(),
            SOLID => r.take(3)?.repeat(pixels),
            PALETTE => {
                let n = usize::from(r.u8()?) + 1;
                if !palette_fits(n, pixels) {
                    return Err(Error::InvalidValue("palette size"));
                }
                let colors = r.take(3 * n)?;
                let indices = r.take(pixels)?;
                let mut rgb = Vec::with_capacity(3 * pixels);
                for &i in indices {
                    let i = usize::from(i);
                    if i >= n {
                        return Err(Error::InvalidValue("palette index"));
                    }
                    rgb.extend_from_slice(&colors[3 * i..3 * i + 3]);
                }
                rgb
            }
            _ => return Err(Error::InvalidValue("tile record")),
        };
        out.push(rgb);
    }
    r.finish()?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(w: u16, h: u16) -> Rect {
        Rect { x: 0, y: 0, w, h }
    }

    #[test]
    fn each_record_kind_decodes_to_rgb_rows() {
        let rects = [rect(2, 1), rect(2, 2), rect(3, 2)];
        let mut raw = vec![RGB, 1, 2, 3, 4, 5, 6, SOLID, 9, 8, 7, PALETTE, 1, 10, 10, 10, 20, 20, 20];
        raw.extend([0, 1, 1, 0, 0, 1]);
        let tiles = decode(&rects, &raw).unwrap();
        assert_eq!(tiles[0], [1, 2, 3, 4, 5, 6]);
        assert_eq!(tiles[1], [9, 8, 7].repeat(4));
        assert_eq!(tiles[2][..6], [10, 10, 10, 20, 20, 20]);
        let (shortest, longest) = bounds(&rects);
        assert!(shortest <= raw.len() && raw.len() <= longest);
    }

    #[test]
    fn malformed_records_are_refused() {
        let one = [rect(2, 2)];
        assert_eq!(decode(&one, &[SOLID, 1, 2]), Err(Error::Truncated));
        assert_eq!(decode(&one, &[SOLID, 1, 2, 3, 0]), Err(Error::TrailingBytes));
        assert_eq!(decode(&one, &[7, 1, 2, 3]), Err(Error::InvalidValue("tile record")));
        // An index past the palette, and a palette larger than the rectangle in RGB.
        assert_eq!(
            decode(&one, &[PALETTE, 1, 0, 0, 0, 1, 1, 1, 0, 2, 0, 0]),
            Err(Error::InvalidValue("palette index"))
        );
        assert!(!palette_fits(4, 4) && palette_fits(2, 4) && !palette_fits(1, 64));
        assert_eq!(decode(&one, &[PALETTE, 3]), Err(Error::InvalidValue("palette size")));
    }
}
