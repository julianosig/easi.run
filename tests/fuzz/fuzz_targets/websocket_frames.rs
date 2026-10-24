//! The admin's WebSocket framing, server side, with its message and frame bounds: any bytes from a client either
//! yield bounded binary messages or end the connection, without a panic.
#![no_main]
use std::io::{Cursor, Read, Write};

use libfuzzer_sys::fuzz_target;
use tungstenite::protocol::{Role, WebSocket, WebSocketConfig};

struct Stream(Cursor<Vec<u8>>);

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The admin's own message and frame bound (`admin/src/api.rs`, where it upgrades a viewer's WebSocket): change both
/// together.
const LIMIT: usize = display_protocol::MAX_CLIENT_MESSAGE + 16;

fuzz_target!(|data: &[u8]| {
    let config = WebSocketConfig::default().max_message_size(Some(LIMIT)).max_frame_size(Some(LIMIT));
    let mut socket = WebSocket::from_raw_socket(Stream(Cursor::new(data.to_vec())), Role::Server, Some(config));
    for _ in 0..64 {
        match socket.read() {
            Ok(tungstenite::Message::Binary(bytes)) => assert!(bytes.len() <= LIMIT),
            Ok(_) => {}
            Err(_) => break,
        }
    }
});
