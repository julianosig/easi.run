//! Any bytes, from a client or from the server, either decode to a message from that side that re-encodes to exactly
//! the same bytes, or are refused.
#![no_main]
use display_protocol::{Direction, Message};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    for direction in [Direction::ClientToServer, Direction::ServerToClient] {
        if let Ok(message) = Message::decode(data, direction) {
            assert_eq!(message.encode(), data);
            assert!(message.direction() == direction);
        }
    }
});
