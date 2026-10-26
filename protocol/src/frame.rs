//! Framing on the Unix socket between the admin application and the display server.
//!
//! A frame is a little-endian `u32` body length followed by the body. The body's first byte says
//! what it carries: a protocol [`Message`] relayed for one viewer, or a link-control record: the
//! admin application's, and the display's answer to opening a page. Each connection serves one viewer.

use crate::{Direction, Error, MAX_CLIENT_MESSAGE, MAX_SERVER_MESSAGE, Message, Reader, Writer};

const RELAY: u8 = 0;
const ATTACH: u8 = 1;
const GRANT: u8 = 2;
const SETTINGS: u8 = 3;
const OPEN_PAGE: u8 = 4;
const PAGE_OPENED: u8 = 5;
const MEDIA_DISCARDED: u8 = 6;
const PATH_CHANGED: u8 = 7;
/// The longest address an `OpenPage` carries.
pub const MAX_PAGE_URL: usize = 2048;

/// The encoder settings a viewer starts with, and the ranges a `Settings` link frame may carry.
pub const DEFAULT_BITRATE_KBPS: u32 = 2500;
pub const DEFAULT_MAX_FPS: u8 = 60;
pub const BITRATE_KBPS: std::ops::RangeInclusive<u32> = 100..=50_000;
pub const MAX_FPS: std::ops::RangeInclusive<u8> = 1..=120;

/// The sizes an output may take, in pixels, both even: what the protocol's video carries. AV1 and H.264 are announced
/// at level 4.0 (see the crate's codecs), whose pictures hold at most 2,359,296 (AV1) and 2,097,152 (H.264) pixels, and
/// past that AV1 would still write level 4.0 while openh264 raises its level by itself. The display checks its output
/// by this rule, and the admin its `ADMIN_OUTPUT`; the controller, which does not depend on this crate, keeps its own
/// copy (`controller/src/request.rs`, `validate`).
pub const OUTPUT_WIDTH: std::ops::RangeInclusive<u32> = 320..=1920;
pub const OUTPUT_HEIGHT: std::ops::RangeInclusive<u32> = 240..=1080;

/// Whether an output of `width` x `height` follows [`OUTPUT_WIDTH`] and [`OUTPUT_HEIGHT`], both even.
pub fn output_size_ok(width: u32, height: u32) -> bool {
    OUTPUT_WIDTH.contains(&width)
        && OUTPUT_HEIGHT.contains(&height)
        && width.is_multiple_of(2)
        && height.is_multiple_of(2)
}

/// The codec a viewer's stream should use when its client can decode it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum CodecChoice {
    /// The best codec the client can decode: AV1, then H.264.
    Auto = 0,
    Av1 = 1,
    H264 = 2,
    /// Lossless tiles only.
    TilesOnly = 3,
}

/// What travels on the admin <-> display link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Link {
    /// A protocol message for (or from) this connection's viewer.
    Relay(Message),
    /// Admin -> display, first frame: who this viewer is.
    Attach { viewer: u64 },
    /// Admin -> display: the control lease. `holder` is true only on the holder's connection; every
    /// grant carries a new `lease_epoch`, and input fenced by any other epoch is refused.
    Grant { lease_epoch: u32, holder: bool },
    /// Admin -> display: this viewer's encoding settings. A codec change starts a new configuration epoch.
    Settings { codec: CodecChoice, bitrate_kbps: u32, max_fps: u8 },
    /// Admin -> display, on the lease holder's connection at `lease_epoch` only: open `url` (https) in the browser,
    /// in place of every page it shows. The admin chooses the address (a public demo's page); no viewer can.
    OpenPage { request: u32, lease_epoch: u32, url: String },
    /// Display -> admin: whether the browser opened request `request`'s page.
    PageOpened { request: u32, opened: bool },
    /// Admin -> display: the network simulator lost the `Video` or `Tiles` message of `revision`, so no acknowledgement
    /// will come for it.
    MediaDiscarded { revision: u64 },
    /// Admin -> display: this viewer's media changed paths (its direct path failed, see the crate's docs): whatever
    /// was in flight will not be acknowledged, the measurements of the old path no longer hold, and the client needs a
    /// new epoch.
    PathChanged,
}

/// Fills in the length in front of a frame body written after four placeholder bytes.
fn framed(mut frame: Vec<u8>) -> Vec<u8> {
    let len = (frame.len() - 4) as u32;
    frame[..4].copy_from_slice(&len.to_le_bytes());
    frame
}

impl Link {
    pub fn encode(&self) -> Vec<u8> {
        // The length goes in front once the body is written.
        let mut body = vec![0; 4];
        let mut w = Writer(&mut body);
        match self {
            Link::Relay(message) => return Link::relay(message),
            Link::Attach { viewer } => {
                w.u8(ATTACH);
                w.u64(*viewer);
            }
            Link::Grant { lease_epoch, holder } => {
                w.u8(GRANT);
                w.u32(*lease_epoch);
                w.u8(u8::from(*holder));
            }
            Link::Settings { codec, bitrate_kbps, max_fps } => {
                w.u8(SETTINGS);
                w.u8(*codec as u8);
                w.u32(*bitrate_kbps);
                w.u8(*max_fps);
            }
            Link::OpenPage { request, lease_epoch, url } => {
                assert!(page_url(url), "an https address of at most MAX_PAGE_URL bytes");
                w.u8(OPEN_PAGE);
                w.u32(*request);
                w.u32(*lease_epoch);
                w.bytes(url.as_bytes());
            }
            Link::PageOpened { request, opened } => {
                w.u8(PAGE_OPENED);
                w.u32(*request);
                w.u8(u8::from(*opened));
            }
            Link::MediaDiscarded { revision } => {
                w.u8(MEDIA_DISCARDED);
                w.u64(*revision);
            }
            Link::PathChanged => w.u8(PATH_CHANGED),
        }
        framed(body)
    }

    /// One relayed message's frame, written straight from the message (no copy of it, and one buffer sized up
    /// front): what the display sends each viewer for every frame.
    pub fn relay(message: &Message) -> Vec<u8> {
        let mut body = Vec::with_capacity(5 + message.size_hint());
        body.extend_from_slice(&[0; 4]);
        body.push(RELAY);
        message.encode_into(&mut body);
        framed(body)
    }

    /// Decodes one frame body that travelled in `direction`. Each link-control record travels one way: from the admin
    /// application to the display (`ClientToServer`), but `PageOpened` back (`ServerToClient`).
    pub fn decode(body: &[u8], direction: Direction) -> Result<Link, Error> {
        let mut r = Reader(body);
        let tag = r.u8().map_err(|_| Error::Empty)?;
        let link = match (tag, direction) {
            (RELAY, _) => return Ok(Link::Relay(Message::decode(r.rest(), direction)?)),
            (ATTACH, Direction::ClientToServer) => Link::Attach { viewer: r.u64()? },
            (GRANT, Direction::ClientToServer) => Link::Grant { lease_epoch: r.u32()?, holder: r.bool()? },
            (SETTINGS, Direction::ClientToServer) => {
                let codec = match r.u8()? {
                    0 => CodecChoice::Auto,
                    1 => CodecChoice::Av1,
                    2 => CodecChoice::H264,
                    3 => CodecChoice::TilesOnly,
                    _ => return Err(Error::InvalidValue("codec choice")),
                };
                let (bitrate_kbps, max_fps) = (r.u32()?, r.u8()?);
                if !BITRATE_KBPS.contains(&bitrate_kbps) || !MAX_FPS.contains(&max_fps) {
                    return Err(Error::InvalidValue("settings"));
                }
                Link::Settings { codec, bitrate_kbps, max_fps }
            }
            (OPEN_PAGE, Direction::ClientToServer) => {
                let (request, lease_epoch) = (r.u32()?, r.u32()?);
                let url = std::str::from_utf8(r.rest()).map_err(|_| Error::InvalidValue("utf-8"))?;
                if !page_url(url) {
                    return Err(Error::InvalidValue("page address"));
                }
                Link::OpenPage { request, lease_epoch, url: url.to_owned() }
            }
            (PAGE_OPENED, Direction::ServerToClient) => Link::PageOpened { request: r.u32()?, opened: r.bool()? },
            (MEDIA_DISCARDED, Direction::ClientToServer) => Link::MediaDiscarded { revision: r.u64()? },
            (PATH_CHANGED, Direction::ClientToServer) => Link::PathChanged,
            _ => return Err(Error::UnknownKind(tag)),
        };
        r.finish()?;
        Ok(link)
    }
}

/// An address the browser may be told to open: https, printable ASCII, at most [`MAX_PAGE_URL`] bytes.
pub fn page_url(url: &str) -> bool {
    url.len() <= MAX_PAGE_URL && url.starts_with("https://") && url.bytes().all(|b| b.is_ascii_graphic())
}

/// Largest frame body accepted in `direction`.
pub fn max_body(direction: Direction) -> usize {
    1 + match direction {
        Direction::ClientToServer => MAX_CLIENT_MESSAGE,
        Direction::ServerToClient => MAX_SERVER_MESSAGE,
    }
}

/// Largest number of bytes a caller may push at once; read from the socket in chunks of at most this size.
pub const MAX_CHUNK: usize = 64 * 1024;

/// Incremental frame splitter. It never holds more than one partial frame plus one chunk: the caller pushes a chunk
/// of at most [`MAX_CHUNK`] bytes and then takes every complete frame before pushing again.
#[derive(Debug)]
pub struct FrameReader {
    buf: Vec<u8>,
    start: usize,
    max: usize,
}

impl FrameReader {
    pub fn new(direction: Direction) -> FrameReader {
        FrameReader { buf: Vec::new(), start: 0, max: max_body(direction) }
    }

    /// Appends one received chunk. Fails on an oversized chunk, on more retained bytes than one frame and one chunk,
    /// and as soon as the buffered frame header declares a body over the bound.
    pub fn push(&mut self, bytes: &[u8]) -> Result<(), Error> {
        if bytes.len() > MAX_CHUNK || self.retained() + bytes.len() > 4 + self.max + MAX_CHUNK {
            return Err(Error::TooLarge);
        }
        if self.start > 0 {
            self.buf.drain(..self.start);
            self.start = 0;
        }
        self.buf.extend_from_slice(bytes);
        self.check_head()
    }

    /// Returns the next complete frame body, if one is buffered.
    pub fn next_body(&mut self) -> Result<Option<Vec<u8>>, Error> {
        self.check_head()?;
        let Some(len) = self.declared() else { return Ok(None) };
        if self.retained() < 4 + len {
            return Ok(None);
        }
        let body = self.buf[self.start + 4..self.start + 4 + len].to_vec();
        self.start += 4 + len;
        self.check_head()?;
        Ok(Some(body))
    }

    fn retained(&self) -> usize {
        self.buf.len() - self.start
    }

    fn declared(&self) -> Option<usize> {
        let head = self.buf.get(self.start..self.start + 4)?;
        Some(u32::from_le_bytes(head.try_into().expect("four bytes")) as usize)
    }

    fn check_head(&self) -> Result<(), Error> {
        match self.declared() {
            Some(len) if len > self.max => Err(Error::TooLarge),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::output_size_ok;

    #[test]
    fn output_sizes_stay_within_what_the_announced_video_level_carries() {
        for (w, h) in [(1280, 800), (320, 240), (1920, 1080)] {
            assert!(output_size_ok(w, h), "{w}x{h}");
        }
        for (w, h) in [(1281, 800), (1280, 801), (318, 240), (320, 238), (1922, 1080), (1920, 1082), (2560, 1440)] {
            assert!(!output_size_ok(w, h), "{w}x{h}");
        }
    }
}
