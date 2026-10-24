//! The I420 picture both video encoders take: BT.709 studio range, which browsers assume for an HD stream that does
//! not say (OpenH264 writes no color description), and whose primaries are sRGB's, so drawing a frame needs no gamut
//! conversion.

use yuvutils_rs::{
    YuvChromaSubsampling, YuvConversionMode, YuvPlanarImageMut, YuvRange, YuvStandardMatrix, bgra_to_yuv420,
};

/// One output-sized I420 picture, refilled from each XRGB8888 frame.
pub struct Yuv {
    pub planes: YuvPlanarImageMut<'static, u8>,
    pub width: usize,
    pub height: usize,
}

impl Yuv {
    pub fn new(width: u32, height: u32) -> Yuv {
        Yuv {
            planes: YuvPlanarImageMut::alloc(width, height, YuvChromaSubsampling::Yuv420),
            width: width as usize,
            height: height as usize,
        }
    }

    /// Converts an XRGB8888 picture whose rows lie `stride` pixels apart.
    pub fn convert(&mut self, pixels: &[u32], stride: usize) -> Result<(), String> {
        if stride < self.width {
            return Err("color conversion: rows overlap".into());
        }
        let bytes: &[u8] = bytemuck::cast_slice(pixels);
        bgra_to_yuv420(
            &mut self.planes,
            bytes,
            stride as u32 * 4,
            YuvRange::Limited,
            YuvStandardMatrix::Bt709,
            YuvConversionMode::Balanced,
        )
        .map_err(|e| format!("color conversion: {e:?}"))
    }
}
