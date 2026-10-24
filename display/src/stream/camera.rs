//! A person's camera picture in a recording: the JPEG their viewer sends (`RecordCamera`), decoded to RGB within a bound
//! on its sides. Any lease holder can send one (in the public demo, any visitor), so this is fuzzed on its own
//! (`tests/fuzz/fuzz_targets/camera_picture.rs` includes this file).

use zune_core::bytestream::ZCursor;
use zune_core::colorspace::ColorSpace;
use zune_core::options::DecoderOptions;

/// The largest camera picture decoded, in pixels per side (the viewer sends 480x360).
pub const MAX_SIDE: usize = 640;

/// A decoded picture: RGB bytes, `width` by `height`, neither side zero nor past [`MAX_SIDE`].
pub struct Rgb {
    pub rgb: Vec<u8>,
    pub width: usize,
    pub height: usize,
}

/// Decodes `jpeg`, or `None` for a broken, empty or oversized picture (refused from its header, before its pixels are
/// allocated).
pub fn decode(jpeg: &[u8]) -> Option<Rgb> {
    let options = DecoderOptions::default()
        .set_max_width(MAX_SIDE)
        .set_max_height(MAX_SIDE)
        .jpeg_set_out_colorspace(ColorSpace::RGB);
    let mut decoder = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(jpeg), options);
    let rgb = decoder.decode().ok()?;
    let info = decoder.info()?;
    let (width, height) = (usize::from(info.width), usize::from(info.height));
    if width == 0 || height == 0 || rgb.len() != width * height * 3 {
        return None;
    }
    Some(Rgb { rgb, width, height })
}
