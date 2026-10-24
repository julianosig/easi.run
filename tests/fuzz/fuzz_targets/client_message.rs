//! Any bytes from a client either decode to a message that re-encodes to exactly the same bytes, or are refused.
#![no_main]
use display_protocol::{Direction, Message};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(message) = Message::decode(data, Direction::ClientToServer) {
        assert_eq!(message.encode(), data);
        assert!(message.direction() == Direction::ClientToServer);
    }
});
