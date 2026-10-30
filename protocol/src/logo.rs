//! The operator's logo, which each display shows on its background in place of easi.run's: a PNG the controller reads
//! from its `CONTROLLER_DISPLAY_LOGO_FILE` and passes to every display as `DISPLAY_LOGO`, the base64 of its bytes. Both
//! check it with [`decode`] at start, and refuse to start on one it refuses.

use base64::Engine as _;

/// The most bytes a logo's PNG may hold.
pub const MAX_BYTES: usize = 64 * 1024;
/// The most pixels a logo may have on each side.
pub const MAX_SIDE: u32 = 1024;

/// A decoded logo: `width` × `height` pixels, straight RGBA, row by row.
#[derive(Debug, PartialEq, Eq)]
pub struct Logo {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// A logo's PNG as `DISPLAY_LOGO` carries it.
pub fn encode(png: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(png)
}

/// The logo `DISPLAY_LOGO` carries, decoded as [`decode`] does.
pub fn decode_text(text: &str) -> Result<Logo, String> {
    // Within the bound before decoding: 4 base64 characters for each 3 bytes.
    if text.len() > MAX_BYTES.div_ceil(3) * 4 {
        return Err(format!("larger than {MAX_BYTES} bytes"));
    }
    let png = base64::engine::general_purpose::STANDARD.decode(text).map_err(|e| format!("not base64: {e}"))?;
    decode(&png)
}

/// `png` decoded whole: at most [`MAX_BYTES`], an 8-bit RGB or RGBA PNG at most [`MAX_SIDE`] on each side, every chunk's
/// CRC checked, and its image data and its end held to [`exact`]. Else why not, in words.
pub fn decode(png: &[u8]) -> Result<Logo, String> {
    if png.len() > MAX_BYTES {
        return Err(format!("larger than {MAX_BYTES} bytes"));
    }
    let not_png = |e: png::DecodingError| format!("not a valid PNG: {e}");
    // Its own default ignores the Adler-32 and an ancillary chunk's bad CRC.
    let mut options = png::DecodeOptions::default();
    options.set_ignore_checksums(false);
    options.set_skip_ancillary_crc_failures(false);
    let mut decoder = png::Decoder::new_with_options(std::io::Cursor::new(png), options);
    // An RGB PNG's transparent color (its tRNS chunk) comes out as alpha: the PNG comes out RGBA.
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().map_err(not_png)?;
    let info = reader.info();
    let (width, height, interlaced) = (info.width, info.height, info.interlaced);
    if width > MAX_SIDE || height > MAX_SIDE {
        return Err(format!("{width}x{height} pixels, more than {MAX_SIDE} on a side"));
    }
    let channels = match (info.color_type, info.bit_depth) {
        (png::ColorType::Rgb, png::BitDepth::Eight) => 3,
        (png::ColorType::Rgba, png::BitDepth::Eight) => 4,
        (color, depth) => return Err(format!("{color:?} at {depth:?} bits; it must be 8-bit RGB or RGBA")),
    };
    let rgb = reader.output_color_type() == (png::ColorType::Rgb, png::BitDepth::Eight);
    let mut pixels = vec![0; reader.output_buffer_size().ok_or("too large to decode")?];
    reader.next_frame(&mut pixels).map_err(not_png)?;
    reader.finish().map_err(not_png)?;
    exact(png, width, height, channels, interlaced).map_err(|why| format!("not a valid PNG: {why}"))?;
    let rgba =
        if rgb { pixels.as_chunks::<3>().0.iter().flat_map(|&[r, g, b]| [r, g, b, 0xff]).collect() } else { pixels };
    Ok(Logo { width, height, rgba })
}

/// Whether the PNG the png crate decoded (so its chunks are whole and in order) is exact where that crate lets things
/// go: nothing follows its IEND chunk, and its image data is one zlib stream that ends, with the right Adler-32, inflates
/// to exactly the filtered rows of a `width` × `height` image of `channels` bytes a pixel (in its seven passes when
/// `interlaced`), and has nothing after it.
fn exact(png: &[u8], width: u32, height: u32, channels: usize, interlaced: bool) -> Result<(), String> {
    use miniz_oxide::inflate::TINFLStatus;
    use miniz_oxide::inflate::core::{DecompressorOxide, decompress, inflate_flags};
    let (mut at, mut data) = (8, Vec::new());
    loop {
        let field = |from: usize, len: usize| png.get(from..from + len).ok_or("a chunk cut short");
        let len = u32::from_be_bytes(field(at, 4)?.try_into().expect("four bytes")) as usize;
        let kind = field(at + 4, 4)?;
        if kind == b"IDAT" {
            data.extend_from_slice(field(at + 8, len)?);
        }
        at += 12 + len;
        if kind == b"IEND" {
            break;
        }
    }
    if at != png.len() {
        return Err("bytes after its IEND chunk".into());
    }
    let rows = |w: u32, h: u32| if w == 0 { 0 } else { h as usize * (1 + w as usize * channels) };
    let size = if interlaced {
        const ADAM7: [(u32, u32, u32, u32); 7] =
            [(0, 0, 8, 8), (4, 0, 8, 8), (0, 4, 4, 8), (2, 0, 4, 4), (0, 2, 2, 4), (1, 0, 2, 2), (0, 1, 1, 2)];
        let pass = |(x, y, dx, dy): (u32, u32, u32, u32)| {
            rows(width.saturating_sub(x).div_ceil(dx), height.saturating_sub(y).div_ceil(dy))
        };
        ADAM7.into_iter().map(pass).sum()
    } else {
        rows(width, height)
    };
    // One byte more than the rows: a stream that fills it holds more than the image.
    let mut out = vec![0; size + 1];
    let flags = inflate_flags::TINFL_FLAG_PARSE_ZLIB_HEADER
        | inflate_flags::TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF
        | inflate_flags::TINFL_FLAG_COMPUTE_ADLER32;
    match decompress(&mut DecompressorOxide::new(), &data, &mut out, 0, flags) {
        (TINFLStatus::Done, read, written) if written == size && read == data.len() => Ok(()),
        (TINFLStatus::Done, _, written) if written == size => Err("bytes after its image data's zlib stream".into()),
        (TINFLStatus::Done | TINFLStatus::HasMoreOutput, _, _) => {
            Err("image data that inflates to more or less than the image".into())
        }
        (status, _, _) => Err(format!("image data whose zlib stream does not end whole ({status:?})")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `width` × `height` PNG of `color` and `depth` whose pixels are `data`.
    pub fn png(width: u32, height: u32, color: png::ColorType, depth: png::BitDepth, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(color);
        encoder.set_depth(depth);
        encoder.write_header().unwrap().write_image_data(data).unwrap();
        out
    }

    /// A chunk of `kind` holding `data`, with its CRC.
    fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut crc = flate2::Crc::new();
        crc.update(kind);
        crc.update(data);
        [&(data.len() as u32).to_be_bytes()[..], kind, data, &crc.sum().to_be_bytes()].concat()
    }

    /// A 2 × 2 8-bit RGB PNG whose image data is the zlib stream `idat`, with `before` (chunks) ahead of it and `after`
    /// (any bytes) past its IEND.
    fn crafted(before: &[u8], idat: &[u8], after: &[u8]) -> Vec<u8> {
        let header = [&2u32.to_be_bytes()[..], &2u32.to_be_bytes(), &[8, 2, 0, 0, 0]].concat();
        let signature = b"\x89PNG\r\n\x1a\n";
        [&signature[..], &chunk(b"IHDR", &header), before, &chunk(b"IDAT", idat), &chunk(b"IEND", &[]), after].concat()
    }

    /// The zlib stream of `raw`, whole.
    fn zlib(raw: &[u8]) -> Vec<u8> {
        use std::io::Write;
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(raw).unwrap();
        encoder.finish().unwrap()
    }

    /// Two rows of two pixels, each with its filter byte (none).
    const ROWS: [u8; 14] = [0, 1, 2, 3, 4, 5, 6, 0, 7, 8, 9, 10, 11, 12];

    #[test]
    fn a_crafted_png_decodes_only_when_every_check_holds() {
        let good = crafted(&[], &zlib(&ROWS), &[]);
        assert_eq!(decode(&good).unwrap().rgba, [1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255, 10, 11, 12, 255]);
        let refused = |png: &[u8], what: &str| assert!(decode(png).is_err(), "{what} was accepted");
        // An ancillary chunk whose CRC is wrong.
        let mut text = chunk(b"tEXt", b"Title\0logo");
        *text.last_mut().unwrap() ^= 1;
        refused(&crafted(&text, &zlib(&ROWS), &[]), "a bad ancillary CRC");
        // The zlib stream's Adler-32 wrong, or left out.
        let mut adler = zlib(&ROWS);
        *adler.last_mut().unwrap() ^= 1;
        refused(&crafted(&[], &adler, &[]), "a wrong Adler-32");
        let stream = zlib(&ROWS);
        refused(&crafted(&[], &stream[..stream.len() - 4], &[]), "a missing Adler-32");
        // A stream that never ends: flushed, its last block never marked last.
        let mut open = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut open, &ROWS).unwrap();
        std::io::Write::flush(&mut open).unwrap();
        refused(&crafted(&[], open.get_ref(), &[]), "an unfinished zlib stream");
        // More decompressed data than the image holds, and bytes past the stream's end.
        refused(&crafted(&[], &zlib(&[&ROWS[..], &[0; 7]].concat()), &[]), "extra decompressed data");
        refused(&crafted(&[], &[&zlib(&ROWS)[..], &[0; 4]].concat(), &[]), "bytes past the zlib stream");
        // Anything after IEND.
        refused(&crafted(&[], &zlib(&ROWS), b"garbage"), "bytes after IEND");
        // Too little data, and an image whose stream is split over two IDAT chunks, which is fine.
        refused(&crafted(&[], &zlib(&ROWS[..10]), &[]), "missing decompressed data");
        let stream = zlib(&ROWS);
        let split = crafted(&chunk(b"IDAT", &stream[..5]), &stream[5..], &[]);
        assert_eq!(decode(&split).unwrap().rgba, decode(&good).unwrap().rgba);
    }

    #[test]
    fn an_rgb_png_s_transparent_color_comes_out_transparent() {
        let trns = chunk(b"tRNS", &[0, 4, 0, 5, 0, 6]);
        let logo = decode(&crafted(&trns, &zlib(&ROWS), &[])).unwrap();
        assert_eq!(logo.rgba, [1, 2, 3, 255, 4, 5, 6, 0, 7, 8, 9, 255, 10, 11, 12, 255]);
    }

    #[test]
    fn an_interlaced_png_decodes_to_the_same_pixels() {
        // Adam7 over 2 × 2: the first pass holds the top left pixel, the sixth the top right, the seventh the bottom row.
        let passes = [0, 1, 2, 3, 0, 4, 5, 6, 0, 7, 8, 9, 10, 11, 12];
        let mut interlaced = crafted(&[], &zlib(&passes), &[]);
        interlaced[28] = 1;
        let crc = chunk(b"IHDR", &interlaced[16..29]);
        interlaced[8..33].copy_from_slice(&crc);
        assert_eq!(decode(&interlaced).unwrap().rgba, decode(&crafted(&[], &zlib(&ROWS), &[])).unwrap().rgba);
        assert!(decode(&crafted(&[], &zlib(&passes), &[])).is_err(), "the same data, not interlaced");
    }

    #[test]
    fn an_8_bit_rgb_or_rgba_png_decodes_to_straight_rgba() {
        let rgba = [1, 2, 3, 4, 5, 6, 7, 8];
        let logo = decode(&png(2, 1, png::ColorType::Rgba, png::BitDepth::Eight, &rgba)).unwrap();
        assert_eq!(logo, Logo { width: 2, height: 1, rgba: rgba.to_vec() });
        let logo = decode_text(&encode(&png(2, 1, png::ColorType::Rgb, png::BitDepth::Eight, &[1, 2, 3, 4, 5, 6])));
        assert_eq!(logo.unwrap().rgba, [1, 2, 3, 255, 4, 5, 6, 255]);
    }

    #[test]
    fn anything_else_is_refused() {
        let side = MAX_SIDE + 1;
        let wide = png(side, 1, png::ColorType::Rgb, png::BitDepth::Eight, &vec![0; side as usize * 3]);
        assert!(decode(&wide).unwrap_err().contains("on a side"));
        let tall = png(1, side, png::ColorType::Rgb, png::BitDepth::Eight, &vec![0; side as usize * 3]);
        assert!(decode(&tall).unwrap_err().contains("on a side"));
        let deep = png(1, 1, png::ColorType::Rgba, png::BitDepth::Sixteen, &[0; 8]);
        assert!(decode(&deep).unwrap_err().contains("8-bit RGB or RGBA"));
        let gray = png(1, 1, png::ColorType::Grayscale, png::BitDepth::Eight, &[0]);
        assert!(decode(&gray).unwrap_err().contains("8-bit RGB or RGBA"));
        assert!(decode(b"GIF89a").unwrap_err().contains("not a valid PNG"));
        let mut cut = png(4, 4, png::ColorType::Rgb, png::BitDepth::Eight, &[7; 48]);
        cut.truncate(cut.len() - 20);
        assert!(decode(&cut).unwrap_err().contains("not a valid PNG"), "a stream cut short");
        assert!(decode(&vec![0; MAX_BYTES + 1]).unwrap_err().contains("larger than"));
        assert!(decode_text("not base64!").unwrap_err().contains("not base64"));
        assert!(decode_text(&"A".repeat(MAX_BYTES / 3 * 4 + 8)).unwrap_err().contains("larger than"));
    }
}
