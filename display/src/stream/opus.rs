//! The browser's sound as Opus (libopus 1.5.2 built from source and statically linked; see scripts/opus.sh): 20 ms
//! packets of 48 kHz stereo, one encoder for every viewer, and for a recording an encoder of its own and a decoder of
//! the person's microphone.
//!
//! With `av1`, this module holds the display's `unsafe` code: seven functions of libopus, each with the invariants
//! stated beside it.
#![allow(unsafe_code)]

use std::ffi::c_int;

use display_protocol::{AUDIO_CHANNELS, AUDIO_FRAME_US, AUDIO_SAMPLE_RATE, MAX_AUDIO_PACKET};

/// Samples per channel in one packet.
pub const FRAME_SAMPLES: usize = (AUDIO_SAMPLE_RATE as u64 * AUDIO_FRAME_US / 1_000_000) as usize;
/// Bytes of one 16-bit sample of every channel: sound moves in whole ones.
pub const SAMPLE_BYTES: usize = AUDIO_CHANNELS as usize * 2;
/// Bytes of 16-bit interleaved stereo in one packet.
pub const FRAME_BYTES: usize = FRAME_SAMPLES * SAMPLE_BYTES;
/// Samples of every channel, interleaved, in one packet.
pub const FRAME_PCM: usize = FRAME_SAMPLES * AUDIO_CHANNELS as usize;
/// One packet's 16-bit samples, interleaved.
pub type Pcm = [i16; FRAME_PCM];

const BITRATE: c_int = 64_000;
/// Middle of libopus's 0-10 scale: transparent for a browser's sound at a fraction of the top setting's CPU.
const COMPLEXITY: c_int = 5;

const OPUS_OK: c_int = 0;
const OPUS_APPLICATION_AUDIO: c_int = 2049;
const OPUS_SET_BITRATE_REQUEST: c_int = 4002;
const OPUS_SET_COMPLEXITY_REQUEST: c_int = 4010;
const OPUS_GET_LOOKAHEAD_REQUEST: c_int = 4027;

#[repr(C)]
struct OpusEncoder {
    _private: [u8; 0],
}

#[repr(C)]
struct OpusDecoderState {
    _private: [u8; 0],
}

unsafe extern "C" {
    fn opus_encoder_create(fs: i32, channels: c_int, application: c_int, error: *mut c_int) -> *mut OpusEncoder;
    fn opus_encoder_ctl(st: *mut OpusEncoder, request: c_int, ...) -> c_int;
    fn opus_encode(st: *mut OpusEncoder, pcm: *const i16, frame_size: c_int, data: *mut u8, max: i32) -> i32;
    fn opus_encoder_destroy(st: *mut OpusEncoder);
    fn opus_decoder_create(fs: i32, channels: c_int, error: *mut c_int) -> *mut OpusDecoderState;
    fn opus_decode(
        st: *mut OpusDecoderState,
        data: *const u8,
        len: i32,
        pcm: *mut i16,
        frame_size: c_int,
        decode_fec: c_int,
    ) -> c_int;
    fn opus_decoder_destroy(st: *mut OpusDecoderState);
}

pub struct Opus {
    handle: *mut OpusEncoder,
}

// SAFETY: the encoder state is only reached through `&mut self`, from one thread at a time.
unsafe impl Send for Opus {}

impl Opus {
    pub fn new() -> Result<Opus, String> {
        Opus::with_bitrate(BITRATE)
    }

    /// An encoder at `bitrate` bits a second (a recording's sound wants more than the live stream's).
    pub fn with_bitrate(bitrate: c_int) -> Result<Opus, String> {
        let mut error: c_int = 0;
        // SAFETY: plain arguments and a valid out pointer; a null handle is checked below.
        let handle = unsafe {
            opus_encoder_create(AUDIO_SAMPLE_RATE as i32, AUDIO_CHANNELS as c_int, OPUS_APPLICATION_AUDIO, &mut error)
        };
        if handle.is_null() || error != OPUS_OK {
            return Err(format!("opus: the encoder did not start ({error})"));
        }
        let opus = Opus { handle };
        opus.ctl(OPUS_SET_BITRATE_REQUEST, bitrate)?;
        opus.ctl(OPUS_SET_COMPLEXITY_REQUEST, COMPLEXITY)?;
        Ok(opus)
    }

    fn ctl(&self, request: c_int, value: c_int) -> Result<(), String> {
        // SAFETY: the handle is live (destroyed only in Drop), and both requests take one `opus_int32` argument.
        match unsafe { opus_encoder_ctl(self.handle, request, value) } {
            OPUS_OK => Ok(()),
            e => Err(format!("opus: request {request} failed ({e})")),
        }
    }

    /// The samples the encoder delays its output by (a decoder skips them: Ogg and Matroska's pre-skip).
    pub fn lookahead(&self) -> Result<u16, String> {
        let mut samples: i32 = 0;
        // SAFETY: the handle is live, and the request takes one `opus_int32 *` it writes.
        match unsafe { opus_encoder_ctl(self.handle, OPUS_GET_LOOKAHEAD_REQUEST, &mut samples as *mut i32) } {
            OPUS_OK => u16::try_from(samples).map_err(|_| "opus: an out-of-range lookahead".to_owned()),
            e => Err(format!("opus: the lookahead request failed ({e})")),
        }
    }

    /// One packet from exactly one frame of interleaved stereo samples.
    pub fn encode(&mut self, pcm: &Pcm) -> Result<Vec<u8>, String> {
        let mut out = vec![0u8; MAX_AUDIO_PACKET];
        // SAFETY: the handle is live; `pcm` holds FRAME_SAMPLES samples per channel and `out` MAX_AUDIO_PACKET writable
        // bytes, as the call is told.
        let n = unsafe {
            opus_encode(self.handle, pcm.as_ptr(), FRAME_SAMPLES as c_int, out.as_mut_ptr(), MAX_AUDIO_PACKET as i32)
        };
        if n <= 0 {
            return Err(format!("opus: encoding failed ({n})"));
        }
        out.truncate(n as usize);
        Ok(out)
    }
}

impl Drop for Opus {
    fn drop(&mut self) {
        // SAFETY: the handle came from a successful create and is destroyed exactly once.
        unsafe { opus_encoder_destroy(self.handle) }
    }
}

/// A decoder of 48 kHz Opus to interleaved stereo (a mono packet comes out on both channels): a person's microphone
/// in a recording.
pub struct Decoder {
    handle: *mut OpusDecoderState,
}

// SAFETY: the decoder state is only reached through `&mut self`, from one thread at a time.
unsafe impl Send for Decoder {}

impl Decoder {
    pub fn new() -> Result<Decoder, String> {
        let mut error: c_int = 0;
        // SAFETY: plain arguments and a valid out pointer; a null handle is checked below.
        let handle = unsafe { opus_decoder_create(AUDIO_SAMPLE_RATE as i32, AUDIO_CHANNELS as c_int, &mut error) };
        if handle.is_null() || error != OPUS_OK {
            return Err(format!("opus: the decoder did not start ({error})"));
        }
        Ok(Decoder { handle })
    }

    /// One packet of at most one frame (20 ms) to interleaved stereo samples; a longer or broken packet is an error.
    pub fn decode(&mut self, packet: &[u8]) -> Result<Vec<i16>, String> {
        if packet.is_empty() || packet.len() > MAX_AUDIO_PACKET {
            return Err("opus: a packet of the wrong size".into());
        }
        let mut pcm = vec![0i16; FRAME_PCM];
        // SAFETY: the handle is live; `packet` holds `len` readable bytes and `pcm` room for FRAME_SAMPLES samples per
        // channel, as the call is told (libopus refuses a packet longer than that).
        let n = unsafe {
            opus_decode(self.handle, packet.as_ptr(), packet.len() as i32, pcm.as_mut_ptr(), FRAME_SAMPLES as c_int, 0)
        };
        if n < 0 {
            return Err(format!("opus: decoding failed ({n})"));
        }
        pcm.truncate(n as usize * AUDIO_CHANNELS as usize);
        Ok(pcm)
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        // SAFETY: the handle came from a successful create and is destroyed exactly once.
        unsafe { opus_decoder_destroy(self.handle) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tone_encodes_to_bounded_packets_and_silence_to_tiny_ones() {
        let mut opus = Opus::new().unwrap();
        let mut tone = [0i16; FRAME_PCM];
        for (i, pair) in tone.chunks_mut(2).enumerate() {
            let s = ((i as f64 * 2.0 * std::f64::consts::PI * 440.0 / 48_000.0).sin() * 8000.0) as i16;
            pair.copy_from_slice(&[s, s]);
        }
        for _ in 0..10 {
            let packet = opus.encode(&tone).unwrap();
            assert!((1..=MAX_AUDIO_PACKET).contains(&packet.len()));
        }
        let silence = Opus::new().unwrap().encode(&[0; FRAME_PCM]).unwrap();
        assert!(silence.len() < 16, "{} bytes of silence", silence.len());
    }

    #[test]
    fn a_packet_decodes_to_one_frame_and_nonsense_is_refused() {
        let mut opus = Opus::with_bitrate(128_000).unwrap();
        assert!((0..2_000).contains(&opus.lookahead().unwrap()));
        let packet = opus.encode(&[1000; FRAME_PCM]).unwrap();
        let mut decoder = Decoder::new().unwrap();
        assert_eq!(decoder.decode(&packet).unwrap().len(), FRAME_PCM);
        assert!(decoder.decode(&[]).is_err());
        assert!(decoder.decode(&[0u8; MAX_AUDIO_PACKET + 1]).is_err());
    }
}
