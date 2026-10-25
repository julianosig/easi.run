//! H.264 Constrained Baseline of the whole output (OpenH264 built from source), from compositor-owned pixels.

use openh264::OpenH264API;
use openh264::encoder::{
    BitRate, Complexity, Encoder, EncoderConfig, FrameRate, IntraFramePeriod, Level, Profile, RateControlMode,
    UsageType,
};
use openh264::formats::YUVSource;
use yuvutils_rs::YuvPlanarImageMut;

use super::yuv::Yuv;

pub struct H264 {
    encoder: Encoder,
    yuv: Yuv,
}

struct Planes<'a>(&'a YuvPlanarImageMut<'static, u8>, usize, usize);

impl YUVSource for Planes<'_> {
    fn dimensions(&self) -> (usize, usize) {
        (self.1, self.2)
    }

    fn strides(&self) -> (usize, usize, usize) {
        (self.0.y_stride as usize, self.0.u_stride as usize, self.0.v_stride as usize)
    }

    fn y(&self) -> &[u8] {
        self.0.y_plane.borrow()
    }

    fn u(&self) -> &[u8] {
        self.0.u_plane.borrow()
    }

    fn v(&self) -> &[u8] {
        self.0.v_plane.borrow()
    }
}

impl H264 {
    pub fn new(width: u32, height: u32, bitrate_kbps: u32, fps: u8) -> Result<H264, String> {
        let config = EncoderConfig::new()
            .usage_type(UsageType::ScreenContentRealTime)
            .profile(Profile::Baseline)
            .level(Level::Level_4_0)
            .complexity(Complexity::Low)
            .rate_control_mode(RateControlMode::Bitrate)
            .bitrate(BitRate::from_bps(bitrate_kbps * 1000))
            .max_frame_rate(FrameRate::from_hz(f32::from(fps)))
            .skip_frames(false)
            .scene_change_detect(false)
            .intra_frame_period(IntraFramePeriod::from_num_frames(0))
            .num_threads(1);
        let encoder =
            Encoder::with_api_config(OpenH264API::from_source(), config).map_err(|e| format!("openh264: {e}"))?;
        Ok(H264 { encoder, yuv: Yuv::new(width, height) })
    }

    /// Encodes one XRGB8888 frame to an Annex B access unit; a keyframe carries its SPS and PPS.
    pub fn encode(&mut self, pixels: &[u32], keyframe: bool) -> Result<Vec<u8>, String> {
        self.yuv.convert(pixels, self.yuv.width)?;
        if keyframe {
            self.encoder.force_intra_frame();
        }
        let source = Planes(&self.yuv.planes, self.yuv.width, self.yuv.height);
        let stream = self.encoder.encode(&source).map_err(|e| format!("openh264: {e}"))?;
        Ok(stream.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whether an Annex B access unit holds an IDR slice (NAL type 5).
    fn is_idr(annex_b: &[u8]) -> bool {
        annex_b.windows(4).any(|w| w[..3] == [0, 0, 1] && w[3] & 0x1f == 5)
    }

    #[test]
    fn keyframes_carry_parameter_sets_and_inter_frames_are_small() {
        let (w, h) = (320u32, 240u32);
        let mut encoder = H264::new(w, h, 1000, 30).unwrap();
        let frame: Vec<u32> = (0..w * h).map(|i| 0xff00_0000 | ((i % 251) * 0x010101)).collect();
        let key = encoder.encode(&frame, true).unwrap();
        assert!(is_idr(&key));
        let nal_types: Vec<u8> = key.windows(4).filter(|w| w[..3] == [0, 0, 1]).map(|w| w[3] & 0x1f).collect();
        assert!(nal_types.contains(&7) && nal_types.contains(&8), "SPS and PPS in {nal_types:?}");
        let next = encoder.encode(&frame, false).unwrap();
        assert!(!is_idr(&next));
        assert!(next.len() < key.len());
    }
}
