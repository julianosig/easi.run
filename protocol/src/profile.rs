//! Framing of a saved profile's archive between the admin application and the browser's launcher, over the standard
//! streams of the controller's two profile commands (`controller/src/template.rs`, `Exec`).
//!
//! A frame is a one-byte kind and a big-endian `u32` length, then that many bytes: [`Frame::Data`], a piece of the
//! archive (a tar stream) of at most [`MAX_DATA`] bytes; [`Frame::End`], empty, after the last piece of an archive
//! sent whole, or, from the launcher after a restore, to say it took it; or [`Frame::Failed`], why the sender stopped,
//! at most [`MAX_FAILED`] bytes of UTF-8. A stream that ends before its end frame, or holds anything else, is a
//! failure: an archive is used only once its end arrived.

/// How long one profile's archive may be, as a tar stream: Chrome evicts what sites store beyond 256 MiB together (the
/// launcher's `StorageQuotaSettings`), and a profile that had opened sixteen major sites measured 26 MiB as an archive
/// (its files took 38 MiB with the caches it leaves out), so this leaves the sites' share twice over for cookies,
/// history and every other file. The admin refuses to save a longer one, and the launcher to unpack one.
pub const MAX_ARCHIVE: u64 = 512 * 1024 * 1024;
/// The most bytes a data frame carries.
pub const MAX_DATA: usize = 64 * 1024;
/// The most bytes a failure frame carries.
pub const MAX_FAILED: usize = 1024;
pub const HEADER: usize = 5;

const DATA: u8 = b'd';
const END: u8 = b'e';
const FAILED: u8 = b'x';

/// One frame's header: its kind and its body's length.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Frame {
    Data(usize),
    End,
    Failed(usize),
}

impl Frame {
    /// The header that starts it.
    pub fn header(self) -> [u8; HEADER] {
        let (kind, length) = match self {
            Frame::Data(length) => (DATA, length.min(MAX_DATA)),
            Frame::End => (END, 0),
            Frame::Failed(length) => (FAILED, length.min(MAX_FAILED)),
        };
        let mut header = [kind, 0, 0, 0, 0];
        header[1..].copy_from_slice(&(length as u32).to_be_bytes());
        header
    }

    /// The frame `header` starts, if it is one: a known kind within its bound.
    pub fn parse(header: [u8; HEADER]) -> Result<Frame, String> {
        let length = u32::from_be_bytes([header[1], header[2], header[3], header[4]]) as usize;
        match header[0] {
            DATA if length <= MAX_DATA => Ok(Frame::Data(length)),
            END if length == 0 => Ok(Frame::End),
            FAILED if length <= MAX_FAILED => Ok(Frame::Failed(length)),
            DATA | END | FAILED => Err(format!("a profile frame of {length} bytes is out of bounds")),
            kind => Err(format!("no profile frame starts with {kind:#04x}")),
        }
    }
}

/// `why` as a failure's body: cut to [`MAX_FAILED`] bytes, at a character's boundary.
pub fn failure(why: &str) -> &[u8] {
    let mut end = why.len().min(MAX_FAILED);
    while !why.is_char_boundary(end) {
        end -= 1;
    }
    &why.as_bytes()[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_frame_parses_back_and_nothing_else_does() {
        for frame in [Frame::Data(0), Frame::Data(MAX_DATA), Frame::End, Frame::Failed(3), Frame::Failed(MAX_FAILED)] {
            assert_eq!(Frame::parse(frame.header()), Ok(frame));
        }
        assert_eq!(Frame::Data(7).header(), [b'd', 0, 0, 0, 7]);
        assert_eq!(Frame::End.header(), [b'e', 0, 0, 0, 0]);
        for bad in [
            [b'd', 0, 1, 0, 1],
            [b'e', 0, 0, 0, 1],
            [b'x', 0, 0, 4, 1],
            [b'D', 0, 0, 0, 0],
            [0, 0, 0, 0, 0],
            [b'd', 0xff, 0xff, 0xff, 0xff],
        ] {
            assert!(Frame::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_failure_is_cut_at_its_bound_on_a_character() {
        assert_eq!(failure("short"), b"short");
        let long = "é".repeat(MAX_FAILED);
        let cut = failure(&long);
        assert!(cut.len() <= MAX_FAILED && std::str::from_utf8(cut).is_ok());
        assert_eq!(cut.len(), MAX_FAILED, "two-byte characters fill it exactly");
    }
}
