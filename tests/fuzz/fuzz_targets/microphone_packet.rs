//! A recording's microphone as the display decodes it (`display/src/stream/opus.rs`, included, over libopus): any
//! packets a lease holder can send (`RecordMicrophone`), through one decoder as a recording does, are each refused or
//! decoded to at most one 20 ms frame of interleaved stereo. The input is the packets, each after its length in two
//! big-endian bytes.
#![no_main]
use libfuzzer_sys::fuzz_target;

#[allow(dead_code)] // the encoder half of the module, which only the display uses
#[path = "../../../display/src/stream/opus.rs"]
mod opus;

// The display links libopus in its build script; here only this target does (see build.rs for where it comes from).
// It declares nothing, so it is the one `unsafe` this workspace allows outside the included module.
#[allow(unsafe_code)]
#[link(name = "opus", kind = "static")]
unsafe extern "C" {}

fuzz_target!(|data: &[u8]| {
    let mut decoder = opus::Decoder::new().expect("a decoder starts");
    let mut rest = data;
    while let [high, low, tail @ ..] = rest {
        let (packet, next) = tail.split_at(usize::from(u16::from_be_bytes([*high, *low])).min(tail.len()));
        rest = next;
        if let Ok(pcm) = decoder.decode(packet) {
            assert!(pcm.len() <= opus::FRAME_SAMPLES * 2 && pcm.len() % 2 == 0, "{} samples", pcm.len());
        }
    }
});
