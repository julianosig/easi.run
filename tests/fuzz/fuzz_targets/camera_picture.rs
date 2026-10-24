//! A recording's camera as the display decodes it (`display/src/stream/camera.rs`, included): any picture a lease
//! holder can send (`RecordCamera`, up to its size bound) is refused or decodes to RGB within the side bound.
#![no_main]
use display_protocol::MAX_RECORD_PICTURE;
use libfuzzer_sys::fuzz_target;

#[path = "../../../display/src/stream/camera.rs"]
mod camera;

fuzz_target!(|data: &[u8]| {
    if data.is_empty() || data.len() > MAX_RECORD_PICTURE {
        return;
    }
    if let Some(picture) = camera::decode(data) {
        assert!((1..=camera::MAX_SIDE).contains(&picture.width), "{} pixels wide", picture.width);
        assert!((1..=camera::MAX_SIDE).contains(&picture.height), "{} pixels high", picture.height);
        assert_eq!(picture.rgb.len(), 3 * picture.width * picture.height);
    }
});
