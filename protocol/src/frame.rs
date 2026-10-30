//! Framing on the Unix socket between the admin application and the display server.
//!
//! A frame is a little-endian `u32` body length followed by the body. The body's first byte says
//! what it carries: a protocol [`Message`] relayed for one viewer, or a link-control record: the
//! admin application's, and the display's answer to opening a page. Each connection serves one viewer.

use crate::{Direction, Error, Message, Reader, Writer, https_ascii_url};

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
/// How long an `OpenPage` may wait for its answer before it is given up: by the admin for the display's
/// `PageOpened`, by the display for the launcher's, and by the launcher for Chromium's.
pub const PAGE_OPEN_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

/// The ranges a viewer's [`Settings`] may take.
pub const BITRATE_KBPS: std::ops::RangeInclusive<u32> = 100..=50_000;
pub const MAX_FPS: std::ops::RangeInclusive<u8> = 1..=120;

/// The sizes an output may take, in pixels, both even: what the protocol's video carries. AV1 and H.264 are announced
/// at level 4.0 (see the crate's codecs), whose pictures hold at most 2,359,296 (AV1) and 2,097,152 (H.264) pixels, and
/// past that AV1 would still write level 4.0 while openh264 raises its level by itself. The display checks its output
/// by this rule ([`output_size_ok`]), the admin its `ADMIN_OUTPUT` and every size asked for, and the controller every
/// session's (`controller/src/request.rs`, `validate`).
pub const OUTPUT_WIDTH: std::ops::RangeInclusive<u32> = 320..=1920;
pub const OUTPUT_HEIGHT: std::ops::RangeInclusive<u32> = 240..=1080;

/// Whether an output of `width` x `height` follows [`OUTPUT_WIDTH`] and [`OUTPUT_HEIGHT`], both even, and is not one
/// SVT-AV1 crashes on (see [`svt_av1_crashes`]).
pub fn output_size_ok(width: u32, height: u32) -> bool {
    OUTPUT_WIDTH.contains(&width)
        && OUTPUT_HEIGHT.contains(&height)
        && width.is_multiple_of(2)
        && height.is_multiple_of(2)
        && !svt_av1_crashes(width, height)
}

/// A `WIDTHxHEIGHT` output size that follows [`output_size_ok`], as the admin, the display, and the browser's launcher
/// are configured with one.
pub fn parse_output_size(text: &str) -> Option<(u32, u32)> {
    let (width, height) = text.split_once('x')?;
    let (width, height) = (width.parse().ok()?, height.parse().ok()?);
    output_size_ok(width, height).then_some((width, height))
}

/// The size rule [`output_size_ok`] holds, in words, for the messages that refuse a size.
pub const OUTPUT_SIZE_RULE: &str = "even, within 320x240 and 1920x1080, and not one SVT-AV1 crashes on (one side 2 to 8 pixels past a multiple of 64, the other 10 to 16)";

/// Whether SVT-AV1 3.1.2 crashes encoding a `width` x `height` picture: when, past their last whole 64-pixel
/// superblock, one side has 2 to 8 pixels left and the other 10 to 16 (every even pair mapped, a page left unmapped
/// past each plane; 1488x962 is one). A video region is the output's size, or whole tiles of 64 clipped at its edges,
/// so its sides leave what the output's leave, or none: an output that passes keeps every region clear of it.
pub fn svt_av1_crashes(width: u32, height: u32) -> bool {
    let (small, large) = (2..=8, 10..=16);
    let (w, h) = (width % 64, height % 64);
    (small.contains(&w) && large.contains(&h)) || (large.contains(&w) && small.contains(&h))
}

wire_enum! {
    /// The codec a viewer's stream should use when its client can decode it.
    #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize), serde(rename_all = "snake_case"))]
    pub enum CodecChoice ("codec choice") {
        /// The best codec the client can decode: AV1, then H.264.
        Auto = 0,
        Av1 = 1,
        H264 = 2,
        /// Lossless tiles only.
        TilesOnly = 3,
    }
}

/// A viewer's encoding settings, as the admin sets them and a `Settings` link frame carries them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Settings {
    pub codec: CodecChoice,
    pub bitrate_kbps: u32,
    pub max_fps: u8,
}

impl Settings {
    /// Whether the bitrate and the frame-rate cap are within [`BITRATE_KBPS`] and [`MAX_FPS`].
    pub fn valid(&self) -> bool {
        BITRATE_KBPS.contains(&self.bitrate_kbps) && MAX_FPS.contains(&self.max_fps)
    }
}

/// What a viewer starts with.
impl Default for Settings {
    fn default() -> Settings {
        Settings { codec: CodecChoice::Auto, bitrate_kbps: 2500, max_fps: 60 }
    }
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
    Settings(Settings),
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
            Link::Settings(settings) => {
                w.u8(SETTINGS);
                w.u8(settings.codec as u8);
                w.u32(settings.bitrate_kbps);
                w.u8(settings.max_fps);
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
                let settings =
                    Settings { codec: CodecChoice::from_wire(r.u8()?)?, bitrate_kbps: r.u32()?, max_fps: r.u8()? };
                if !settings.valid() {
                    return Err(Error::InvalidValue("settings"));
                }
                Link::Settings(settings)
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
    https_ascii_url(url, MAX_PAGE_URL)
}

/// Largest frame body accepted in `direction`.
pub fn max_body(direction: Direction) -> usize {
    1 + direction.max_message_size()
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
    use super::{output_size_ok, parse_output_size};

    #[test]
    fn output_sizes_stay_within_what_the_announced_video_level_carries() {
        for (w, h) in [(1280, 800), (320, 240), (1920, 1080), (1440, 900), (1366, 768), (1680, 1050), (1488, 970)] {
            assert!(output_size_ok(w, h), "{w}x{h}");
        }
        for (w, h) in [
            (1281, 800),
            (1280, 801),
            (318, 240),
            (320, 238),
            (1922, 1080),
            (1920, 1082),
            (2560, 1440),
            (1488, 962),
            (1104, 900),
        ] {
            assert!(!output_size_ok(w, h), "{w}x{h}");
        }
    }

    #[test]
    fn an_output_size_is_read_as_width_x_height_and_follows_the_rule() {
        assert_eq!(parse_output_size("1280x800"), Some((1280, 800)));
        for bad in ["1281x800", "1922x1080", "1488x962", "1280", "x800", "-2x4", "1280x800x2", ""] {
            assert_eq!(parse_output_size(bad), None, "{bad}");
        }
    }
}
