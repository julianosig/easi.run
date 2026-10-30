//! The admin-display link: arbitrary chunks into the frame reader, every complete body decoded both ways (relayed
//! messages, attach, lease grants, encoder settings, and injection lines with their answers). The reader must stay within its bound.
#![no_main]
use display_protocol::Direction;
use display_protocol::frame::{FrameReader, Link, MAX_CHUNK, max_body};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    for direction in [Direction::ClientToServer, Direction::ServerToClient] {
        let mut reader = FrameReader::new(direction);
        // The first byte picks the chunk size, so frames split at every boundary.
        let size = usize::from(data.first().copied().unwrap_or(1)).clamp(1, MAX_CHUNK);
        'chunks: for chunk in data.chunks(size) {
            if reader.push(chunk).is_err() {
                break;
            }
            loop {
                match reader.next_body() {
                    Ok(Some(body)) => {
                        assert!(body.len() <= max_body(direction));
                        if let Ok(link) = Link::decode(&body, direction) {
                            assert_eq!(&link.encode()[4..], &body[..]);
                        }
                    }
                    Ok(None) => break,
                    Err(_) => break 'chunks,
                }
            }
        }
    }
});
