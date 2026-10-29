//! easi.run wire protocol, version [`VERSION`].
//!
//! One message is one WebSocket binary frame between the person's browser and the admin
//! application, and one length-prefixed frame (see [`frame`]) between the admin application and
//! the display server. Every message starts with a one-byte kind followed by fixed little-endian
//! fields. The set is closed: an unknown kind, a short or over-long body, an out-of-range value,
//! or invalid UTF-8 is an error and the receiver closes the stream.
//!
//! Media is normative:
//! - `CODEC_H264` means H.264 Constrained Baseline, level 4.0 (`avc1.42E028`), Annex B byte stream; every keyframe
//!   carries its SPS and PPS.
//! - `CODEC_AV1` means AV1 Main profile, 8-bit, level 4.0 (`av01.0.08M.08`), low-overhead OBU stream; every keyframe
//!   carries its sequence header.
//! - A `Tiles` message carries rectangles at most [`MAX_TILE_SIDE`] on each side and one raw deflate (RFC 1951)
//!   stream of exactly `raw_len` bytes: one [`records`] record per rectangle, in order. A
//!   full refresh larger than one message is a run of `Tiles` messages, each of its own revision.
//! - Cursor images are QOI images (4 channels, straight alpha) of at most 256x256 pixels and
//!   [`MAX_CURSOR_IMAGE`] bytes.
//! - An `Audio` message carries one Opus packet (RFC 6716) of [`AUDIO_FRAME_US`] of 48 kHz stereo, at most
//!   [`MAX_AUDIO_PACKET`] bytes. Packets follow each other without a gap while the browser plays sound; a gap in their
//!   timestamps means sound was skipped (a pause, or sound the display discarded), and both the server's encoder and
//!   the client's decoder go on across it, as across a lost packet.
//!
//! Ordering, per viewer stream: revisions only increase, every `Video`, `Tiles` and `Cursor` message taking a new,
//! greater one (across epochs too), so a video frame and a tile never share one. The client draws a message only over pixels last
//! drawn at a lower revision, and drops anything of an older configuration epoch. `KeyframeRequest` names the epoch it
//! was made in; the server ignores a stale one.
//!
//! Flow: the client acknowledges every `Video`, `Tiles` and `Cursor` message as it arrives, whatever its epoch, with a
//! `MediaAck` of its revision (cumulative: it acknowledges every earlier one too). The server sends them only while
//! what it sent and has not seen acknowledged is young and small enough for the connection (about its round trip plus
//! [`MEDIA_AGE_ALLOWANCE_MS`]), so a slow path gets fewer, current frames rather than a backlog of old ones. An
//! acknowledgement above the highest revision the server sent closes the stream.
//!
//! Direct path: a client that can open a WebTransport session asks for one (`UdpRequest`), and the admin application
//! answers with a `UdpOffer`: the address to open, which carries a single-use ticket, and the SHA-256 of the
//! certificate to accept there. Over that session the server sends `Config`, `Video`, `Tiles` and `Cursor` messages on
//! one unidirectional stream, each framed as on the display link (a little-endian `u32` length, then a zero byte and
//! the message: see [`frame`]), and `Audio` messages as datagrams (one message each); the client sends its `MediaAck`
//! and `KeyframeRequest` messages on one unidirectional stream framed the same way. Everything else stays on the
//! WebSocket. `MediaMoved`, on the WebSocket after its last media message, says the media of offer `generation`
//! continue on the session: the client takes them only after it, and acknowledges none before. `MediaFallback` says
//! they are back on the WebSocket: the client drops whatever media of the session it has not taken, and every media
//! message until the next `Config`, which opens a new epoch.
//!
//! Time: `Video.timestamp_us` (the composition time of that frame), `Audio.timestamp_us` (when the display read the
//! packet's first sample), `TimePong.server_time_us`, and `ServerStats.server_time_us` are one monotonic display clock
//! in microseconds.
//!
//! Input: `Wheel` deltas are DOM-style pixels (positive `dy` scrolls down). `Key` `Down`/`Up` carry a non-zero evdev
//! code and no text; `Text` carries text and code 0. `Clipboard` replaces the browser's clipboard with the text and
//! pastes it once, so the client never sends the paste keystroke itself. `Copied` is text the browser copied, sent only
//! to the lease holder at `lease_epoch`; the client places it on the person's clipboard where the browser lets a page
//! write it, and otherwise offers to on a click. `Cursor.shape` is 0 for hidden, 1-34 for the
//! `wp_cursor_shape_device_v1` shapes in protocol order, and 255 for a custom QOI image at most 256x256; the image is
//! present exactly when the shape is 255.
//!
//! Downloads: `FileOffer` announces a download to the lease holder as soon as the browser starts it, and the admin
//! application follows it with `FileLink`, the single-use ticket of the native download (an HTTPS response the person's
//! browser saves). The admin application itself receives the bytes: each `FileRead` is a credit ("consumed up to
//! `offset`"), and the display sends `FileData` in order as the browser writes it, never beyond `offset` +
//! [`FILE_WINDOW`], every piece [`FILE_CHUNK`] bytes but the last; `FileEnd` follows the last piece with the size, and
//! `FileGone` says the download failed or went away. `FileCancel` stops one. A viewer's own `FileRead` and `FileCancel`
//! are refused by the admin application. Uploads: `UploadStart` names a file and its size, `UploadData` carries it in
//! order from offset 0, at most [`UPLOAD_WINDOW`] bytes ahead of the last `UploadAck`, and `UploadDone` says the
//! browser stored it (or not). `FileRequest` asks the holder for files for the page's file chooser, answered by
//! `FileChoose` with the stored uploads (none: cancelled); `FileDrop` drops stored uploads on the page at a point.
//! While the person drags files over the screen, `FileDrag` carries the point and the files' media types (all a page
//! may see before a drop), so the page shows its drag-over state; `FileDragEnd` ends that when the drag leaves or its
//! uploads fail, and a `FileDrop` replaces it with the stored uploads. Every message of a transfer carries the lease
//! epoch; a lease change ends every transfer of the previous holder.
//!
//! Recording: the lease holder asks the display to record the screen (`RecordStart`, a recording number it chooses,
//! with where its camera goes and how it looks; `RecordLayout` moves or restyles it while it records), sends its camera
//! as JPEG pictures (`RecordCamera`) and its microphone as Opus packets of [`AUDIO_FRAME_US`] of 48 kHz audio
//! (`RecordMicrophone`) while it records, and ends it with `RecordStop`. Their timestamps are on the display's clock
//! (the client maps its own through `TimePong`). The display composes the recording and offers it as a download of
//! unknown size as it is made (`FileOffer` and what follows), ending it at the stop; `RecordState` says it started,
//! ended, or failed. A lease change ends it.

/// A `#[repr(u8)]` enum on the wire, and `from_wire`: the variant a byte names, by the variant's own number, so
/// encoding (`as u8`) and decoding cannot drift apart; any other byte is an invalid `$what`.
macro_rules! wire_enum {
    ($(#[$meta:meta])* pub enum $name:ident ($what:literal) { $($(#[$vmeta:meta])* $variant:ident = $value:literal,)+ }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        #[repr(u8)]
        pub enum $name {
            $($(#[$vmeta])* $variant = $value,)+
        }

        impl $name {
            pub(crate) fn from_wire(v: u8) -> Result<Self, $crate::Error> {
                $(if v == $name::$variant as u8 {
                    return Ok($name::$variant);
                })+
                Err($crate::Error::InvalidValue($what))
            }
        }
    };
}

pub mod browser;
pub mod frame;
pub mod keys;
pub mod profile;
pub mod records;
pub mod session;

use std::fmt;

/// The only protocol version this implementation speaks.
pub const VERSION: u16 = 4;
/// How much longer than the connection's round trip media the server sent may stay unacknowledged before it sends
/// no more, in milliseconds.
pub const MEDIA_AGE_ALLOWANCE_MS: u64 = 150;
/// Largest side of a tile's rectangle, in pixels.
pub const MAX_TILE_SIDE: u16 = 256;
/// Largest inflated size of one `Tiles` message's records, in bytes.
pub const MAX_TILES_RAW: usize = 8 * 1024 * 1024;
/// Largest side of the output a configuration announces, in pixels.
pub const MAX_OUTPUT_SIDE: u16 = 4096;

/// Largest server-to-client message (a video keyframe or a batch of lossless tiles).
pub const MAX_SERVER_MESSAGE: usize = 2 * 1024 * 1024;
/// Largest access unit one `Video` message carries: the server bound less the message's fixed fields.
pub const MAX_VIDEO_PAYLOAD: usize = MAX_SERVER_MESSAGE - 30;
/// Largest client-to-server message (a bounded clipboard payload plus its header).
pub const MAX_CLIENT_MESSAGE: usize = MAX_CLIPBOARD + 16;
/// Largest WebSocket message and frame a viewer may send the admin: one client message with room to spare.
pub const MAX_WEBSOCKET_MESSAGE: usize = MAX_CLIENT_MESSAGE + 16;
/// Largest clipboard text a person may paste, in bytes.
pub const MAX_CLIPBOARD: usize = 64 * 1024;
/// Largest address a `UdpOffer` carries, in bytes.
pub const MAX_UDP_URL: usize = 512;
/// Largest text the browser may copy to the person, in bytes.
pub const MAX_COPIED: usize = 1024 * 1024;
/// Largest piece of a file one `FileData` carries, in bytes.
pub const FILE_CHUNK: usize = 64 * 1024;
/// Download bytes the display may send beyond the receiver's last `FileRead` offset.
pub const FILE_WINDOW: u64 = 4 * 1024 * 1024;
/// Length of a download ticket, in bytes.
pub const FILE_TICKET: usize = 32;
/// Largest file name, in bytes.
pub const MAX_FILE_NAME: usize = 255;
/// Largest upload, in bytes: the display receives no longer one, and the browser's launcher stores none.
pub const MAX_UPLOAD: u64 = 512 * 1024 * 1024;
/// Largest piece of an upload one `UploadData` carries, in bytes (it fits a client message).
pub const UPLOAD_CHUNK: usize = 60 * 1024;
/// Upload bytes a client may send beyond the last `UploadAck`.
pub const UPLOAD_WINDOW: u64 = 256 * 1024;
/// Most uploads one `FileChoose` or `FileDrop` names.
pub const MAX_CHOSEN: usize = 64;
/// The `file://` URI the browser gets for an upload's absolute `path` (chosen or dropped), every byte outside the
/// unreserved set and `/` percent-encoded.
pub fn file_uri(path: &str) -> String {
    let mut uri = String::from("file://");
    for &b in path.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_' | b'.' | b'~') {
            uri.push(b as char);
        } else {
            uri.push_str(&format!("%{b:02X}"));
        }
    }
    uri
}
/// Largest media type a `FileDrag` names, in bytes.
pub const MAX_DRAG_TYPE: usize = 127;
/// Largest composed text carried by one key message, in bytes.
pub const MAX_TEXT: usize = 1024;
/// Largest number of tiles in one tiles message.
pub const MAX_TILES: usize = 4096;
/// Largest custom cursor image, in encoded bytes.
pub const MAX_CURSOR_IMAGE: usize = 64 * 1024;
/// Largest Opus packet (RFC 6716, section 3.4), in bytes.
pub const MAX_AUDIO_PACKET: usize = 1275;
/// The duration of every `Audio` packet, in microseconds.
pub const AUDIO_FRAME_US: u64 = 20_000;
/// The sample rate and channel count of every `Audio` packet.
pub const AUDIO_SAMPLE_RATE: u32 = 48_000;
pub const AUDIO_CHANNELS: u32 = 2;
/// Largest camera picture a `RecordCamera` carries, in bytes (it fits a client message).
pub const MAX_RECORD_PICTURE: usize = 60 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

wire_enum! {
    pub enum Codec ("codec") {
        None = 0,
        Av1 = 1,
        H264 = 2,
    }
}

/// Bit set of the video codecs a client can decode.
pub const CODEC_AV1: u8 = 1 << 0;
pub const CODEC_H264: u8 = 1 << 1;
const KNOWN_CODECS: u8 = CODEC_AV1 | CODEC_H264;

wire_enum! {
    pub enum PointerKind ("pointer kind") {
        Move = 0,
        Down = 1,
        Up = 2,
    }
}

wire_enum! {
    pub enum KeyKind ("key kind") {
        Down = 0,
        Up = 1,
        /// Composed text (an IME commit or a character without a physical key).
        Text = 2,
    }
}

/// Steps of a camera position or side: a share of the screen, from 0 to this.
pub const LAYOUT_UNITS: u16 = 10_000;
/// The camera's smallest side, in [`LAYOUT_UNITS`] of the screen's width.
pub const CAMERA_SIDE_MIN: u16 = 1_000;
/// The camera's largest side, in [`LAYOUT_UNITS`] of the screen's width (the display also keeps it within the
/// screen's height).
pub const CAMERA_SIDE_MAX: u16 = 4_000;
/// The camera's roundest corners, in percent of half its side: a circle.
pub const CAMERA_RADIUS_MAX: u8 = 100;

/// Where the camera is in a recording, and how it looks. The camera is a square: `x` and `y` place its top-left
/// corner, in [`LAYOUT_UNITS`] of the screen's width and height, and `side` is its side, in [`LAYOUT_UNITS`] of the
/// screen's width (from [`CAMERA_SIDE_MIN`] to [`CAMERA_SIDE_MAX`]); the display keeps it inside the screen. `radius`
/// rounds its corners, in percent of half its side (0 square, [`CAMERA_RADIUS_MAX`] a circle).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CameraLayout {
    pub x: u16,
    pub y: u16,
    pub side: u16,
    pub radius: u8,
}

wire_enum! {
    /// Where a recording is.
    pub enum RecordPhase ("recording phase") {
        /// It is being recorded, and offered as `file`.
        Recording = 0,
        /// It ended; its download ends with `FileEnd`.
        Ended = 1,
        /// It could not go on; its download ends with `FileGone`.
        Failed = 2,
    }
}

wire_enum! {
    pub enum LeaseHolder ("lease holder") {
        /// Nobody holds control; the agent may act.
        Agent = 0,
        /// This connection holds control.
        You = 1,
        /// Another person's connection holds control.
        Other = 2,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    /// Client -> server, first message: the protocol version the client speaks, which must be [`VERSION`], and the
    /// codecs it decodes.
    Hello { version: u16, codecs: u8 },
    /// Server -> client: a new configuration; everything of an older epoch is discarded.
    Config { version: u16, epoch: u32, width: u16, height: u16, codec: Codec, max_fps: u8 },
    /// Server -> client: one encoded video frame for the region `rect`.
    Video { epoch: u32, revision: u64, rect: Rect, keyframe: bool, timestamp_us: u64, payload: Vec<u8> },
    /// Server -> client (the lease holder only): text the browser copied.
    Copied { lease_epoch: u32, text: String },
    /// Server -> client (the lease holder only): a download the browser started, and its size when the browser knows
    /// it.
    FileOffer { lease_epoch: u32, file: u32, size: Option<u64>, name: String },
    /// Server -> client (the lease holder only, from the admin application): the ticket that fetches the download.
    FileLink { lease_epoch: u32, file: u32, ticket: [u8; FILE_TICKET] },
    /// Server -> client (the lease holder's admin application): the next piece of a download, at `offset`.
    FileData { lease_epoch: u32, file: u32, offset: u64, data: Vec<u8> },
    /// Server -> client (the lease holder only): every piece of the download was sent; it is `size` bytes.
    FileEnd { lease_epoch: u32, file: u32, size: u64 },
    /// Server -> client (the lease holder only): the download failed or is no longer available.
    FileGone { lease_epoch: u32, file: u32 },
    /// Server -> client (the lease holder only): the page opened a file chooser; answer with `FileChoose`.
    FileRequest { lease_epoch: u32, request: u32, multiple: bool },
    /// Server -> client (the lease holder only): the page closed its file chooser; files chosen for it go nowhere.
    FileRequestEnd { lease_epoch: u32, request: u32 },
    /// Server -> client (the lease holder only): this many bytes of an upload were taken.
    UploadAck { lease_epoch: u32, upload: u32, received: u64 },
    /// Server -> client (the lease holder only): the browser stored the upload, or could not.
    UploadDone { lease_epoch: u32, upload: u32, stored: bool },
    /// Server -> client: one Opus packet of the browser's sound.
    Audio { timestamp_us: u64, payload: Vec<u8> },
    /// Server -> client: lossless tiles, all of one revision.
    Tiles { epoch: u32, revision: u64, rects: Vec<Rect>, raw_len: u32, data: Vec<u8> },
    /// Server -> client: the pointer position and shape (`image` is empty for a named shape).
    Cursor { epoch: u32, revision: u64, x: u16, y: u16, shape: u8, hot_x: u16, hot_y: u16, image: Vec<u8> },
    /// Client -> server: the decoder lost state; answer with a keyframe and a full refresh.
    KeyframeRequest { epoch: u32 },
    /// Client -> server: every `Video`, `Tiles` and `Cursor` message up to `revision` arrived.
    MediaAck { revision: u64 },
    /// Client -> server (to the admin application): an offer of the direct path, which this client can open.
    UdpRequest,
    /// Server -> client (from the admin application): open `url` (https, with its ticket) as a WebTransport session,
    /// accepting only the certificate whose DER has this SHA-256.
    UdpOffer { generation: u32, cert_sha256: [u8; 32], url: String },
    /// Server -> client (from the admin application): the media of offer `generation` continue on its session.
    MediaMoved { generation: u32 },
    /// Server -> client (from the admin application): the media are back on the WebSocket.
    MediaFallback { generation: u32 },
    /// Server -> client: encoder statistics for the last interval.
    ServerStats { server_time_us: u64, frames: u32, bytes: u32, encode_us_p50: u32, encode_us_p95: u32 },
    /// Client -> server: presentation statistics for the last interval; `presented` counts the video frames drawn.
    ClientStats { presented: u32, decode_us_p50: u32, hardware: bool },
    /// Client -> server: clock calibration probe; the server answers with `TimePong`.
    TimePing { client_time_us: u64 },
    /// Server -> client: the echo of a `TimePing` with the server's clock.
    TimePong { client_time_us: u64, server_time_us: u64 },
    /// Server -> client: who holds control, fenced by `lease_epoch`.
    Lease { lease_epoch: u32, holder: LeaseHolder },
    /// Client -> server (control only).
    Pointer { lease_epoch: u32, kind: PointerKind, button: u8, x: u16, y: u16 },
    /// Client -> server (control only): scroll in pixels.
    Wheel { lease_epoch: u32, x: u16, y: u16, dx: i16, dy: i16 },
    /// Client -> server (control only): `code` is a Linux evdev key code; `text` is set only for `Text`.
    Key { lease_epoch: u32, kind: KeyKind, code: u16, text: String },
    /// Client -> server (control only, on a user gesture): plain text to paste.
    Clipboard { lease_epoch: u32, text: String },
    /// Client -> server (control only, from the admin application): the bytes of a download consumed so far; the
    /// display may send up to `offset` + [`FILE_WINDOW`].
    FileRead { lease_epoch: u32, file: u32, offset: u64 },
    /// Client -> server (control only, from the admin application): the download is no longer wanted.
    FileCancel { lease_epoch: u32, file: u32 },
    /// Client -> server (control only): a file the person gives the browser.
    UploadStart { lease_epoch: u32, upload: u32, size: u64, name: String },
    /// Client -> server (control only): the next piece of an upload, at `offset`.
    UploadData { lease_epoch: u32, upload: u32, offset: u64, data: Vec<u8> },
    /// Client -> server (control only): the uploads chosen for a `FileRequest`; none cancels it.
    FileChoose { lease_epoch: u32, request: u32, uploads: Vec<u32> },
    /// Client -> server (control only): stored uploads dropped on the page at (x, y).
    FileDrop { lease_epoch: u32, x: u16, y: u16, uploads: Vec<u32> },
    /// Client -> server (control only): files dragged over (x, y), one media type each (empty when unknown).
    FileDrag { lease_epoch: u32, x: u16, y: u16, types: Vec<String> },
    /// Client -> server (control only): the files' drag left the screen or will not be dropped.
    FileDragEnd { lease_epoch: u32 },
    /// Client -> server (lease holder): record the screen, with the camera as `layout` says and the sounds it names.
    RecordStart {
        lease_epoch: u32,
        recording: u32,
        layout: CameraLayout,
        camera: bool,
        microphone: bool,
        page_sound: bool,
    },
    /// Client -> server (lease holder): one camera picture (JPEG) of the recording, taken at `timestamp_us`.
    RecordCamera { lease_epoch: u32, recording: u32, timestamp_us: u64, jpeg: Vec<u8> },
    /// Client -> server (lease holder): one Opus packet of the microphone, starting at `timestamp_us`.
    RecordMicrophone { lease_epoch: u32, recording: u32, timestamp_us: u64, payload: Vec<u8> },
    /// Client -> server (lease holder): end the recording.
    RecordStop { lease_epoch: u32, recording: u32 },
    /// Client -> server (lease holder): the camera moved or changed its look while recording.
    RecordLayout { lease_epoch: u32, recording: u32, layout: CameraLayout },
    /// Server -> client (the lease holder only): where the recording is; `file` is its download.
    RecordState { lease_epoch: u32, recording: u32, phase: RecordPhase, file: u32, seconds: u32, bytes: u64 },
}

mod kind {
    pub const HELLO: u8 = 0x01;
    pub const CONFIG: u8 = 0x02;
    pub const VIDEO: u8 = 0x03;
    pub const TILES: u8 = 0x04;
    pub const CURSOR: u8 = 0x05;
    pub const KEYFRAME_REQUEST: u8 = 0x06;
    pub const SERVER_STATS: u8 = 0x07;
    pub const CLIENT_STATS: u8 = 0x08;
    pub const TIME_PING: u8 = 0x09;
    pub const TIME_PONG: u8 = 0x0a;
    pub const LEASE: u8 = 0x0b;
    pub const AUDIO: u8 = 0x0c;
    pub const COPIED: u8 = 0x0d;
    pub const FILE_OFFER: u8 = 0x0e;
    pub const FILE_DATA: u8 = 0x0f;
    pub const FILE_REQUEST: u8 = 0x1a;
    pub const UPLOAD_ACK: u8 = 0x1b;
    pub const UPLOAD_DONE: u8 = 0x1c;
    pub const FILE_GONE: u8 = 0x1d;
    pub const FILE_REQUEST_END: u8 = 0x1e;
    pub const FILE_END: u8 = 0x1f;
    pub const FILE_LINK: u8 = 0x20;
    pub const POINTER: u8 = 0x10;
    pub const WHEEL: u8 = 0x11;
    pub const KEY: u8 = 0x12;
    pub const CLIPBOARD: u8 = 0x13;
    pub const FILE_READ: u8 = 0x14;
    pub const UPLOAD_START: u8 = 0x15;
    pub const UPLOAD_DATA: u8 = 0x16;
    pub const FILE_CHOOSE: u8 = 0x17;
    pub const FILE_DROP: u8 = 0x18;
    pub const FILE_CANCEL: u8 = 0x19;
    pub const FILE_DRAG: u8 = 0x21;
    pub const FILE_DRAG_END: u8 = 0x22;
    pub const RECORD_START: u8 = 0x23;
    pub const RECORD_CAMERA: u8 = 0x24;
    pub const RECORD_MICROPHONE: u8 = 0x25;
    pub const RECORD_STOP: u8 = 0x26;
    pub const RECORD_STATE: u8 = 0x27;
    pub const RECORD_LAYOUT: u8 = 0x28;
    pub const MEDIA_ACK: u8 = 0x29;
    pub const UDP_REQUEST: u8 = 0x2a;
    pub const UDP_OFFER: u8 = 0x2b;
    pub const MEDIA_MOVED: u8 = 0x2c;
    pub const MEDIA_FALLBACK: u8 = 0x2d;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Empty,
    UnknownKind(u8),
    Truncated,
    TrailingBytes,
    TooLarge,
    InvalidValue(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Empty => f.write_str("empty message"),
            Error::UnknownKind(k) => write!(f, "unknown message kind 0x{k:02x}"),
            Error::Truncated => f.write_str("truncated message"),
            Error::TrailingBytes => f.write_str("trailing bytes"),
            Error::TooLarge => f.write_str("message too large"),
            Error::InvalidValue(what) => write!(f, "invalid {what}"),
        }
    }
}

impl std::error::Error for Error {}

/// Which side a message may come from; decoding refuses a message from the wrong side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    ClientToServer,
    ServerToClient,
}

impl Message {
    /// The revision a client acknowledges this message by (`MediaAck`): only video, tiles and cursors are media in
    /// flight.
    pub fn media_revision(&self) -> Option<u64> {
        match self {
            Message::Video { revision, .. } | Message::Tiles { revision, .. } | Message::Cursor { revision, .. } => {
                Some(*revision)
            }
            _ => None,
        }
    }

    pub fn direction(&self) -> Direction {
        match self {
            Message::Hello { .. }
            | Message::KeyframeRequest { .. }
            | Message::MediaAck { .. }
            | Message::UdpRequest
            | Message::ClientStats { .. }
            | Message::TimePing { .. }
            | Message::Pointer { .. }
            | Message::Wheel { .. }
            | Message::Key { .. }
            | Message::Clipboard { .. }
            | Message::FileRead { .. }
            | Message::FileCancel { .. }
            | Message::UploadStart { .. }
            | Message::UploadData { .. }
            | Message::FileChoose { .. }
            | Message::FileDrop { .. }
            | Message::FileDrag { .. }
            | Message::FileDragEnd { .. }
            | Message::RecordStart { .. }
            | Message::RecordCamera { .. }
            | Message::RecordMicrophone { .. }
            | Message::RecordStop { .. }
            | Message::RecordLayout { .. } => Direction::ClientToServer,
            Message::Config { .. }
            | Message::Video { .. }
            | Message::Audio { .. }
            | Message::Copied { .. }
            | Message::FileOffer { .. }
            | Message::FileLink { .. }
            | Message::FileData { .. }
            | Message::FileEnd { .. }
            | Message::FileGone { .. }
            | Message::FileRequest { .. }
            | Message::FileRequestEnd { .. }
            | Message::UploadAck { .. }
            | Message::UploadDone { .. }
            | Message::Tiles { .. }
            | Message::Cursor { .. }
            | Message::ServerStats { .. }
            | Message::TimePong { .. }
            | Message::Lease { .. }
            | Message::UdpOffer { .. }
            | Message::MediaMoved { .. }
            | Message::MediaFallback { .. }
            | Message::RecordState { .. } => Direction::ServerToClient,
        }
    }

    /// The lease epoch of a message that belongs to the lease holder alone: input into the browser, transfers, and
    /// recording, both ways. None for every other message, `Lease` included: it tells every viewer who holds control.
    pub fn lease_epoch(&self) -> Option<u32> {
        match self {
            Message::Pointer { lease_epoch, .. }
            | Message::Wheel { lease_epoch, .. }
            | Message::Key { lease_epoch, .. }
            | Message::Clipboard { lease_epoch, .. }
            | Message::FileRead { lease_epoch, .. }
            | Message::FileCancel { lease_epoch, .. }
            | Message::UploadStart { lease_epoch, .. }
            | Message::UploadData { lease_epoch, .. }
            | Message::FileChoose { lease_epoch, .. }
            | Message::FileDrop { lease_epoch, .. }
            | Message::FileDrag { lease_epoch, .. }
            | Message::FileDragEnd { lease_epoch }
            | Message::RecordStart { lease_epoch, .. }
            | Message::RecordCamera { lease_epoch, .. }
            | Message::RecordMicrophone { lease_epoch, .. }
            | Message::RecordStop { lease_epoch, .. }
            | Message::RecordLayout { lease_epoch, .. }
            | Message::Copied { lease_epoch, .. }
            | Message::FileOffer { lease_epoch, .. }
            | Message::FileLink { lease_epoch, .. }
            | Message::FileData { lease_epoch, .. }
            | Message::FileEnd { lease_epoch, .. }
            | Message::FileGone { lease_epoch, .. }
            | Message::FileRequest { lease_epoch, .. }
            | Message::FileRequestEnd { lease_epoch, .. }
            | Message::UploadAck { lease_epoch, .. }
            | Message::UploadDone { lease_epoch, .. }
            | Message::RecordState { lease_epoch, .. } => Some(*lease_epoch),
            Message::Hello { .. }
            | Message::KeyframeRequest { .. }
            | Message::MediaAck { .. }
            | Message::UdpRequest
            | Message::UdpOffer { .. }
            | Message::MediaMoved { .. }
            | Message::MediaFallback { .. }
            | Message::ClientStats { .. }
            | Message::TimePing { .. }
            | Message::Config { .. }
            | Message::Video { .. }
            | Message::Audio { .. }
            | Message::Tiles { .. }
            | Message::Cursor { .. }
            | Message::ServerStats { .. }
            | Message::TimePong { .. }
            | Message::Lease { .. } => None,
        }
    }

    /// True for messages that need the control lease: input into the browser, transfers, and recording.
    pub fn is_input(&self) -> bool {
        self.direction() == Direction::ClientToServer && self.lease_epoch().is_some()
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.size_hint());
        self.encode_into(&mut out);
        out
    }

    /// About how many bytes the message encodes to: its payload and a little for the fields around it, so a buffer
    /// sized by it never grows while the message is written.
    pub fn size_hint(&self) -> usize {
        64 + match self {
            Message::Video { payload, .. } | Message::Audio { payload, .. } => payload.len(),
            Message::RecordMicrophone { payload, .. } => payload.len(),
            Message::Tiles { rects, data, .. } => rects.len() * 8 + data.len(),
            Message::FileData { data, .. } | Message::UploadData { data, .. } => data.len(),
            Message::Cursor { image, .. } => image.len(),
            Message::RecordCamera { jpeg, .. } => jpeg.len(),
            Message::Copied { text, .. } | Message::Clipboard { text, .. } | Message::Key { text, .. } => text.len(),
            Message::FileOffer { name, .. } | Message::UploadStart { name, .. } => name.len(),
            Message::FileChoose { uploads, .. } | Message::FileDrop { uploads, .. } => uploads.len() * 4,
            Message::FileDrag { types, .. } => types.iter().map(|t| t.len() + 2).sum(),
            Message::UdpOffer { url, .. } => url.len(),
            _ => 0,
        }
    }

    /// Appends the message's encoding to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        let mut w = Writer(out);
        match self {
            Message::Hello { version, codecs } => {
                w.u8(kind::HELLO);
                w.u16(*version);
                w.u8(*codecs);
            }
            Message::Config { version, epoch, width, height, codec, max_fps } => {
                w.u8(kind::CONFIG);
                w.u16(*version);
                w.u32(*epoch);
                w.u16(*width);
                w.u16(*height);
                w.u8(*codec as u8);
                w.u8(*max_fps);
            }
            Message::Video { epoch, revision, rect, keyframe, timestamp_us, payload } => {
                w.u8(kind::VIDEO);
                w.u32(*epoch);
                w.u64(*revision);
                w.rect(rect);
                w.u8(u8::from(*keyframe));
                w.u64(*timestamp_us);
                w.bytes(payload);
            }
            Message::Audio { timestamp_us, payload } => {
                assert!(
                    (1..=MAX_AUDIO_PACKET).contains(&payload.len()),
                    "an audio packet is 1 to MAX_AUDIO_PACKET bytes"
                );
                w.u8(kind::AUDIO);
                w.u64(*timestamp_us);
                w.bytes(payload);
            }
            Message::Copied { lease_epoch, text } => {
                assert!(text.len() <= MAX_COPIED, "copied text is at most MAX_COPIED bytes");
                w.u8(kind::COPIED);
                w.u32(*lease_epoch);
                w.bytes(text.as_bytes());
            }
            Message::FileOffer { lease_epoch, file, size, name } => {
                assert!(!name.is_empty() && name.len() <= MAX_FILE_NAME, "a file name is 1 to MAX_FILE_NAME bytes");
                w.u8(kind::FILE_OFFER);
                w.u32(*lease_epoch);
                w.u32(*file);
                w.u8(u8::from(size.is_some()));
                w.u64(size.unwrap_or(0));
                w.bytes(name.as_bytes());
            }
            Message::FileLink { lease_epoch, file, ticket } => {
                w.u8(kind::FILE_LINK);
                w.u32(*lease_epoch);
                w.u32(*file);
                w.bytes(ticket);
            }
            Message::FileEnd { lease_epoch, file, size } => {
                w.u8(kind::FILE_END);
                w.u32(*lease_epoch);
                w.u32(*file);
                w.u64(*size);
            }
            Message::FileData { lease_epoch, file, offset, data } => {
                assert!((1..=FILE_CHUNK).contains(&data.len()), "a file piece is 1 to FILE_CHUNK bytes");
                w.u8(kind::FILE_DATA);
                w.u32(*lease_epoch);
                w.u32(*file);
                w.u64(*offset);
                w.bytes(data);
            }
            Message::FileGone { lease_epoch, file } => {
                w.u8(kind::FILE_GONE);
                w.u32(*lease_epoch);
                w.u32(*file);
            }
            Message::FileRequest { lease_epoch, request, multiple } => {
                w.u8(kind::FILE_REQUEST);
                w.u32(*lease_epoch);
                w.u32(*request);
                w.u8(u8::from(*multiple));
            }
            Message::FileRequestEnd { lease_epoch, request } => {
                w.u8(kind::FILE_REQUEST_END);
                w.u32(*lease_epoch);
                w.u32(*request);
            }
            Message::UploadAck { lease_epoch, upload, received } => {
                w.u8(kind::UPLOAD_ACK);
                w.u32(*lease_epoch);
                w.u32(*upload);
                w.u64(*received);
            }
            Message::UploadDone { lease_epoch, upload, stored } => {
                w.u8(kind::UPLOAD_DONE);
                w.u32(*lease_epoch);
                w.u32(*upload);
                w.u8(u8::from(*stored));
            }
            Message::Tiles { epoch, revision, rects, raw_len, data } => {
                assert!(rects.len() <= MAX_TILES, "a tiles message holds at most MAX_TILES rectangles");
                assert!(
                    rects.iter().all(|r| (1..=MAX_TILE_SIDE).contains(&r.w) && (1..=MAX_TILE_SIDE).contains(&r.h)),
                    "a tile is 1 to MAX_TILE_SIDE pixels on each side"
                );
                w.u8(kind::TILES);
                w.u32(*epoch);
                w.u64(*revision);
                w.u16(rects.len() as u16);
                for rect in rects {
                    w.rect(rect);
                }
                w.u32(*raw_len);
                w.bytes(data);
            }
            Message::Cursor { epoch, revision, x, y, shape, hot_x, hot_y, image } => {
                w.u8(kind::CURSOR);
                w.u32(*epoch);
                w.u64(*revision);
                w.u16(*x);
                w.u16(*y);
                w.u8(*shape);
                w.u16(*hot_x);
                w.u16(*hot_y);
                w.bytes(image);
            }
            Message::KeyframeRequest { epoch } => {
                w.u8(kind::KEYFRAME_REQUEST);
                w.u32(*epoch);
            }
            Message::MediaAck { revision } => {
                w.u8(kind::MEDIA_ACK);
                w.u64(*revision);
            }
            Message::UdpRequest => w.u8(kind::UDP_REQUEST),
            Message::UdpOffer { generation, cert_sha256, url } => {
                assert!(udp_url(url), "an https address of at most MAX_UDP_URL printable bytes");
                w.u8(kind::UDP_OFFER);
                w.u32(*generation);
                w.bytes(cert_sha256);
                w.bytes(url.as_bytes());
            }
            Message::MediaMoved { generation } => {
                w.u8(kind::MEDIA_MOVED);
                w.u32(*generation);
            }
            Message::MediaFallback { generation } => {
                w.u8(kind::MEDIA_FALLBACK);
                w.u32(*generation);
            }
            Message::ServerStats { server_time_us, frames, bytes, encode_us_p50, encode_us_p95 } => {
                w.u8(kind::SERVER_STATS);
                w.u64(*server_time_us);
                w.u32(*frames);
                w.u32(*bytes);
                w.u32(*encode_us_p50);
                w.u32(*encode_us_p95);
            }
            Message::ClientStats { presented, decode_us_p50, hardware } => {
                w.u8(kind::CLIENT_STATS);
                w.u32(*presented);
                w.u32(*decode_us_p50);
                w.u8(u8::from(*hardware));
            }
            Message::TimePing { client_time_us } => {
                w.u8(kind::TIME_PING);
                w.u64(*client_time_us);
            }
            Message::TimePong { client_time_us, server_time_us } => {
                w.u8(kind::TIME_PONG);
                w.u64(*client_time_us);
                w.u64(*server_time_us);
            }
            Message::Lease { lease_epoch, holder } => {
                w.u8(kind::LEASE);
                w.u32(*lease_epoch);
                w.u8(*holder as u8);
            }
            Message::Pointer { lease_epoch, kind: k, button, x, y } => {
                w.u8(kind::POINTER);
                w.u32(*lease_epoch);
                w.u8(*k as u8);
                w.u8(*button);
                w.u16(*x);
                w.u16(*y);
            }
            Message::Wheel { lease_epoch, x, y, dx, dy } => {
                w.u8(kind::WHEEL);
                w.u32(*lease_epoch);
                w.u16(*x);
                w.u16(*y);
                w.u16(*dx as u16);
                w.u16(*dy as u16);
            }
            Message::Key { lease_epoch, kind: k, code, text } => {
                w.u8(kind::KEY);
                w.u32(*lease_epoch);
                w.u8(*k as u8);
                w.u16(*code);
                w.bytes(text.as_bytes());
            }
            Message::Clipboard { lease_epoch, text } => {
                w.u8(kind::CLIPBOARD);
                w.u32(*lease_epoch);
                w.bytes(text.as_bytes());
            }
            Message::FileRead { lease_epoch, file, offset } => {
                w.u8(kind::FILE_READ);
                w.u32(*lease_epoch);
                w.u32(*file);
                w.u64(*offset);
            }
            Message::FileCancel { lease_epoch, file } => {
                w.u8(kind::FILE_CANCEL);
                w.u32(*lease_epoch);
                w.u32(*file);
            }
            Message::UploadStart { lease_epoch, upload, size, name } => {
                assert!(!name.is_empty() && name.len() <= MAX_FILE_NAME, "a file name is 1 to MAX_FILE_NAME bytes");
                w.u8(kind::UPLOAD_START);
                w.u32(*lease_epoch);
                w.u32(*upload);
                w.u64(*size);
                w.bytes(name.as_bytes());
            }
            Message::UploadData { lease_epoch, upload, offset, data } => {
                assert!((1..=UPLOAD_CHUNK).contains(&data.len()), "an upload piece is 1 to UPLOAD_CHUNK bytes");
                w.u8(kind::UPLOAD_DATA);
                w.u32(*lease_epoch);
                w.u32(*upload);
                w.u64(*offset);
                w.bytes(data);
            }
            Message::FileChoose { lease_epoch, request, uploads } => {
                assert!(uploads.len() <= MAX_CHOSEN, "at most MAX_CHOSEN uploads");
                w.u8(kind::FILE_CHOOSE);
                w.u32(*lease_epoch);
                w.u32(*request);
                for upload in uploads {
                    w.u32(*upload);
                }
            }
            Message::FileDrop { lease_epoch, x, y, uploads } => {
                assert!((1..=MAX_CHOSEN).contains(&uploads.len()), "1 to MAX_CHOSEN uploads");
                w.u8(kind::FILE_DROP);
                w.u32(*lease_epoch);
                w.u16(*x);
                w.u16(*y);
                for upload in uploads {
                    w.u32(*upload);
                }
            }
            Message::FileDrag { lease_epoch, x, y, types } => {
                assert!((1..=MAX_CHOSEN).contains(&types.len()), "1 to MAX_CHOSEN types");
                w.u8(kind::FILE_DRAG);
                w.u32(*lease_epoch);
                w.u16(*x);
                w.u16(*y);
                w.u8(types.len() as u8);
                for t in types {
                    assert!(media_type(t), "a media type");
                    w.u8(t.len() as u8);
                    w.bytes(t.as_bytes());
                }
            }
            Message::FileDragEnd { lease_epoch } => {
                w.u8(kind::FILE_DRAG_END);
                w.u32(*lease_epoch);
            }
            Message::RecordStart { lease_epoch, recording, layout, camera, microphone, page_sound } => {
                w.u8(kind::RECORD_START);
                w.u32(*lease_epoch);
                w.u32(*recording);
                write_layout(&mut w, layout);
                w.u8(u8::from(*camera) | u8::from(*microphone) << 1 | u8::from(*page_sound) << 2);
            }
            Message::RecordCamera { lease_epoch, recording, timestamp_us, jpeg } => {
                assert!((1..=MAX_RECORD_PICTURE).contains(&jpeg.len()), "a picture is 1 to MAX_RECORD_PICTURE bytes");
                w.u8(kind::RECORD_CAMERA);
                w.u32(*lease_epoch);
                w.u32(*recording);
                w.u64(*timestamp_us);
                w.bytes(jpeg);
            }
            Message::RecordMicrophone { lease_epoch, recording, timestamp_us, payload } => {
                assert!(
                    (1..=MAX_AUDIO_PACKET).contains(&payload.len()),
                    "an Opus packet is 1 to MAX_AUDIO_PACKET bytes"
                );
                w.u8(kind::RECORD_MICROPHONE);
                w.u32(*lease_epoch);
                w.u32(*recording);
                w.u64(*timestamp_us);
                w.bytes(payload);
            }
            Message::RecordStop { lease_epoch, recording } => {
                w.u8(kind::RECORD_STOP);
                w.u32(*lease_epoch);
                w.u32(*recording);
            }
            Message::RecordLayout { lease_epoch, recording, layout } => {
                w.u8(kind::RECORD_LAYOUT);
                w.u32(*lease_epoch);
                w.u32(*recording);
                write_layout(&mut w, layout);
            }
            Message::RecordState { lease_epoch, recording, phase, file, seconds, bytes } => {
                w.u8(kind::RECORD_STATE);
                w.u32(*lease_epoch);
                w.u32(*recording);
                w.u8(*phase as u8);
                w.u32(*file);
                w.u32(*seconds);
                w.u64(*bytes);
            }
        }
    }

    /// Decodes one message sent in `direction`, refusing anything outside the closed set.
    pub fn decode(bytes: &[u8], direction: Direction) -> Result<Message, Error> {
        let limit = match direction {
            Direction::ClientToServer => MAX_CLIENT_MESSAGE,
            Direction::ServerToClient => MAX_SERVER_MESSAGE,
        };
        if bytes.len() > limit {
            return Err(Error::TooLarge);
        }
        let mut r = Reader(bytes);
        let k = r.u8().map_err(|_| Error::Empty)?;
        let message = match k {
            kind::HELLO => {
                let (version, codecs) = (r.u16()?, r.u8()?);
                // One version on the wire: a client of another one is refused before it gets a configuration.
                if version != VERSION {
                    return Err(Error::InvalidValue("version"));
                }
                if codecs & !KNOWN_CODECS != 0 {
                    return Err(Error::InvalidValue("codec set"));
                }
                Message::Hello { version, codecs }
            }
            kind::CONFIG => {
                let version = r.u16()?;
                let epoch = r.u32()?;
                let (width, height) = (r.u16()?, r.u16()?);
                let codec = Codec::from_wire(r.u8()?)?;
                let max_fps = r.u8()?;
                let size = 1..=MAX_OUTPUT_SIDE;
                if version != VERSION || !size.contains(&width) || !size.contains(&height) || max_fps == 0 {
                    return Err(Error::InvalidValue("configuration"));
                }
                Message::Config { version, epoch, width, height, codec, max_fps }
            }
            kind::VIDEO => Message::Video {
                epoch: r.u32()?,
                revision: r.u64()?,
                rect: r.rect()?,
                keyframe: r.bool()?,
                timestamp_us: r.u64()?,
                payload: r.rest().to_vec(),
            },
            kind::AUDIO => {
                Message::Audio { timestamp_us: r.u64()?, payload: r.sized(MAX_AUDIO_PACKET, "audio packet size")? }
            }
            kind::COPIED => Message::Copied { lease_epoch: r.u32()?, text: utf8(r.rest(), MAX_COPIED)? },
            kind::FILE_OFFER => {
                let (lease_epoch, file, known, size) = (r.u32()?, r.u32()?, r.bool()?, r.u64()?);
                if !known && size != 0 {
                    return Err(Error::InvalidValue("file size"));
                }
                let name = file_name(r.rest())?;
                Message::FileOffer { lease_epoch, file, size: known.then_some(size), name }
            }
            kind::FILE_LINK => {
                let (lease_epoch, file) = (r.u32()?, r.u32()?);
                let ticket = r.take(FILE_TICKET)?.try_into().expect("FILE_TICKET bytes");
                Message::FileLink { lease_epoch, file, ticket }
            }
            kind::FILE_END => Message::FileEnd { lease_epoch: r.u32()?, file: r.u32()?, size: r.u64()? },
            kind::FILE_DATA => {
                let (lease_epoch, file, offset) = (r.u32()?, r.u32()?, r.u64()?);
                Message::FileData { lease_epoch, file, offset, data: r.sized(FILE_CHUNK, "file piece size")? }
            }
            kind::FILE_GONE => Message::FileGone { lease_epoch: r.u32()?, file: r.u32()? },
            kind::FILE_REQUEST => {
                Message::FileRequest { lease_epoch: r.u32()?, request: r.u32()?, multiple: r.bool()? }
            }
            kind::FILE_REQUEST_END => Message::FileRequestEnd { lease_epoch: r.u32()?, request: r.u32()? },
            kind::UPLOAD_ACK => Message::UploadAck { lease_epoch: r.u32()?, upload: r.u32()?, received: r.u64()? },
            kind::UPLOAD_DONE => Message::UploadDone { lease_epoch: r.u32()?, upload: r.u32()?, stored: r.bool()? },
            kind::TILES => {
                let epoch = r.u32()?;
                let revision = r.u64()?;
                let count = usize::from(r.u16()?);
                if count == 0 || count > MAX_TILES {
                    return Err(Error::InvalidValue("tile count"));
                }
                let mut rects = Vec::with_capacity(count);
                for _ in 0..count {
                    let rect = r.rect()?;
                    if rect.w > MAX_TILE_SIDE || rect.h > MAX_TILE_SIDE {
                        return Err(Error::InvalidValue("tile size"));
                    }
                    rects.push(rect);
                }
                let raw_len = r.u32()?;
                let (shortest, longest) = records::bounds(&rects);
                if (raw_len as usize) < shortest || raw_len as usize > longest.min(MAX_TILES_RAW) {
                    return Err(Error::InvalidValue("tile records length"));
                }
                Message::Tiles { epoch, revision, rects, raw_len, data: r.rest().to_vec() }
            }
            kind::CURSOR => {
                let (epoch, revision) = (r.u32()?, r.u64()?);
                let (x, y, shape, hot_x, hot_y) = (r.u16()?, r.u16()?, r.u8()?, r.u16()?, r.u16()?);
                let image = r.rest();
                if image.len() > MAX_CURSOR_IMAGE {
                    return Err(Error::TooLarge);
                }
                // A shape the protocol numbers, and an image exactly for the custom one.
                if !matches!(shape, 0..=34 | 255) || image.is_empty() != (shape != 255) {
                    return Err(Error::InvalidValue("cursor shape"));
                }
                Message::Cursor { epoch, revision, x, y, shape, hot_x, hot_y, image: image.to_vec() }
            }
            kind::KEYFRAME_REQUEST => Message::KeyframeRequest { epoch: r.u32()? },
            kind::MEDIA_ACK => Message::MediaAck { revision: r.u64()? },
            kind::UDP_REQUEST => Message::UdpRequest,
            kind::UDP_OFFER => {
                let generation = r.u32()?;
                let cert_sha256 = r.take(32)?.try_into().expect("32 bytes");
                let url = utf8(r.rest(), MAX_UDP_URL)?;
                if !udp_url(&url) {
                    return Err(Error::InvalidValue("udp address"));
                }
                Message::UdpOffer { generation, cert_sha256, url }
            }
            kind::MEDIA_MOVED => Message::MediaMoved { generation: r.u32()? },
            kind::MEDIA_FALLBACK => Message::MediaFallback { generation: r.u32()? },
            kind::SERVER_STATS => Message::ServerStats {
                server_time_us: r.u64()?,
                frames: r.u32()?,
                bytes: r.u32()?,
                encode_us_p50: r.u32()?,
                encode_us_p95: r.u32()?,
            },
            kind::CLIENT_STATS => {
                Message::ClientStats { presented: r.u32()?, decode_us_p50: r.u32()?, hardware: r.bool()? }
            }
            kind::TIME_PING => Message::TimePing { client_time_us: r.u64()? },
            kind::TIME_PONG => Message::TimePong { client_time_us: r.u64()?, server_time_us: r.u64()? },
            kind::LEASE => Message::Lease { lease_epoch: r.u32()?, holder: LeaseHolder::from_wire(r.u8()?)? },
            kind::POINTER => Message::Pointer {
                lease_epoch: r.u32()?,
                kind: PointerKind::from_wire(r.u8()?)?,
                button: button(r.u8()?)?,
                x: r.u16()?,
                y: r.u16()?,
            },
            kind::WHEEL => Message::Wheel {
                lease_epoch: r.u32()?,
                x: r.u16()?,
                y: r.u16()?,
                dx: r.u16()? as i16,
                dy: r.u16()? as i16,
            },
            kind::KEY => {
                let lease_epoch = r.u32()?;
                let k = KeyKind::from_wire(r.u8()?)?;
                let code = r.u16()?;
                let text = utf8(r.rest(), MAX_TEXT)?;
                if (k == KeyKind::Text) == text.is_empty() || (k == KeyKind::Text) != (code == 0) {
                    return Err(Error::InvalidValue("key"));
                }
                Message::Key { lease_epoch, kind: k, code, text }
            }
            kind::CLIPBOARD => Message::Clipboard { lease_epoch: r.u32()?, text: utf8(r.rest(), MAX_CLIPBOARD)? },
            kind::FILE_READ => Message::FileRead { lease_epoch: r.u32()?, file: r.u32()?, offset: r.u64()? },
            kind::FILE_CANCEL => Message::FileCancel { lease_epoch: r.u32()?, file: r.u32()? },
            kind::UPLOAD_START => {
                let (lease_epoch, upload, size) = (r.u32()?, r.u32()?, r.u64()?);
                let name = file_name(r.rest())?;
                Message::UploadStart { lease_epoch, upload, size, name }
            }
            kind::UPLOAD_DATA => {
                let (lease_epoch, upload, offset) = (r.u32()?, r.u32()?, r.u64()?);
                Message::UploadData { lease_epoch, upload, offset, data: r.sized(UPLOAD_CHUNK, "upload piece size")? }
            }
            kind::FILE_CHOOSE => {
                let (lease_epoch, request) = (r.u32()?, r.u32()?);
                let uploads = upload_ids(r.rest(), 0)?;
                Message::FileChoose { lease_epoch, request, uploads }
            }
            kind::FILE_DROP => {
                let (lease_epoch, x, y) = (r.u32()?, r.u16()?, r.u16()?);
                let uploads = upload_ids(r.rest(), 1)?;
                Message::FileDrop { lease_epoch, x, y, uploads }
            }
            kind::FILE_DRAG => {
                let (lease_epoch, x, y, count) = (r.u32()?, r.u16()?, r.u16()?, r.u8()?);
                if !(1..=MAX_CHOSEN).contains(&usize::from(count)) {
                    return Err(Error::InvalidValue("drag types"));
                }
                let mut types = Vec::with_capacity(count.into());
                for _ in 0..count {
                    let len = r.u8()?;
                    let t = utf8(r.take(len.into())?, MAX_DRAG_TYPE)?;
                    if !media_type(&t) {
                        return Err(Error::InvalidValue("media type"));
                    }
                    types.push(t);
                }
                Message::FileDrag { lease_epoch, x, y, types }
            }
            kind::FILE_DRAG_END => Message::FileDragEnd { lease_epoch: r.u32()? },
            kind::RECORD_START => {
                let (lease_epoch, recording) = (r.u32()?, r.u32()?);
                let layout = read_layout(&mut r)?;
                let flags = r.u8()?;
                if flags & !0b111 != 0 {
                    return Err(Error::InvalidValue("recording flags"));
                }
                Message::RecordStart {
                    lease_epoch,
                    recording,
                    layout,
                    camera: flags & 1 != 0,
                    microphone: flags & 2 != 0,
                    page_sound: flags & 4 != 0,
                }
            }
            kind::RECORD_CAMERA => {
                let (lease_epoch, recording, timestamp_us) = (r.u32()?, r.u32()?, r.u64()?);
                let jpeg = r.sized(MAX_RECORD_PICTURE, "camera picture size")?;
                Message::RecordCamera { lease_epoch, recording, timestamp_us, jpeg }
            }
            kind::RECORD_MICROPHONE => {
                let (lease_epoch, recording, timestamp_us) = (r.u32()?, r.u32()?, r.u64()?);
                let payload = r.sized(MAX_AUDIO_PACKET, "microphone packet size")?;
                Message::RecordMicrophone { lease_epoch, recording, timestamp_us, payload }
            }
            kind::RECORD_STOP => Message::RecordStop { lease_epoch: r.u32()?, recording: r.u32()? },
            kind::RECORD_LAYOUT => {
                let (lease_epoch, recording) = (r.u32()?, r.u32()?);
                Message::RecordLayout { lease_epoch, recording, layout: read_layout(&mut r)? }
            }
            kind::RECORD_STATE => {
                let (lease_epoch, recording) = (r.u32()?, r.u32()?);
                let phase = RecordPhase::from_wire(r.u8()?)?;
                Message::RecordState {
                    lease_epoch,
                    recording,
                    phase,
                    file: r.u32()?,
                    seconds: r.u32()?,
                    bytes: r.u64()?,
                }
            }
            other => return Err(Error::UnknownKind(other)),
        };
        r.finish()?;
        if message.direction() != direction {
            return Err(Error::InvalidValue("direction"));
        }
        Ok(message)
    }
}

/// Buttons follow the DOM `MouseEvent.button` numbering: 0 primary, 1 middle, 2 secondary.
fn button(v: u8) -> Result<u8, Error> {
    if v <= 2 { Ok(v) } else { Err(Error::InvalidValue("button")) }
}

/// A list of upload ids, at least `min` and at most [`MAX_CHOSEN`].
fn upload_ids(bytes: &[u8], min: usize) -> Result<Vec<u32>, Error> {
    if !bytes.len().is_multiple_of(4) || !(min..=MAX_CHOSEN).contains(&(bytes.len() / 4)) {
        return Err(Error::InvalidValue("upload list"));
    }
    Ok(bytes.as_chunks::<4>().0.iter().map(|b| u32::from_le_bytes(*b)).collect())
}

/// Empty (unknown), or a lowercase `type/subtype` of RFC 6838 restricted-name characters, at most [`MAX_DRAG_TYPE`]
/// bytes.
pub fn media_type(t: &str) -> bool {
    let name = |part: &str| {
        !part.is_empty()
            && part.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"!#$&-^_.+".contains(&b))
    };
    t.is_empty() || (t.len() <= MAX_DRAG_TYPE && t.split_once('/').is_some_and(|(a, b)| name(a) && name(b)))
}

/// An address a `UdpOffer` may carry: https, printable ASCII, at most [`MAX_UDP_URL`] bytes.
pub fn udp_url(url: &str) -> bool {
    https_ascii_url(url, MAX_UDP_URL)
}

/// An https address of printable ASCII, at most `max` bytes.
fn https_ascii_url(url: &str, max: usize) -> bool {
    url.len() <= max && url.starts_with("https://") && url.bytes().all(|b| b.is_ascii_graphic())
}

fn utf8(bytes: &[u8], max: usize) -> Result<String, Error> {
    if bytes.len() > max {
        return Err(Error::TooLarge);
    }
    String::from_utf8(bytes.to_vec()).map_err(|_| Error::InvalidValue("utf-8"))
}

/// A file's name, as an offered download or a started upload carries it: UTF-8, of at most [`MAX_FILE_NAME`] bytes, and
/// not empty.
fn file_name(bytes: &[u8]) -> Result<String, Error> {
    let name = utf8(bytes, MAX_FILE_NAME)?;
    if name.is_empty() {
        return Err(Error::InvalidValue("file name"));
    }
    Ok(name)
}

fn write_layout(w: &mut Writer, layout: &CameraLayout) {
    w.u16(layout.x);
    w.u16(layout.y);
    w.u16(layout.side);
    w.u8(layout.radius);
}

fn read_layout(r: &mut Reader) -> Result<CameraLayout, Error> {
    let (x, y) = (r.u16()?, r.u16()?);
    if x > LAYOUT_UNITS || y > LAYOUT_UNITS {
        return Err(Error::InvalidValue("camera position"));
    }
    let side = r.u16()?;
    if !(CAMERA_SIDE_MIN..=CAMERA_SIDE_MAX).contains(&side) {
        return Err(Error::InvalidValue("camera side"));
    }
    let radius = r.u8()?;
    if radius > CAMERA_RADIUS_MAX {
        return Err(Error::InvalidValue("camera radius"));
    }
    Ok(CameraLayout { x, y, side, radius })
}

struct Writer<'a>(&'a mut Vec<u8>);

impl Writer<'_> {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn rect(&mut self, r: &Rect) {
        self.u16(r.x);
        self.u16(r.y);
        self.u16(r.w);
        self.u16(r.h);
    }
    fn bytes(&mut self, b: &[u8]) {
        self.0.extend_from_slice(b);
    }
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        if self.0.len() < n {
            return Err(Error::Truncated);
        }
        let (head, tail) = self.0.split_at(n);
        self.0 = tail;
        Ok(head)
    }
    fn rest(&mut self) -> &'a [u8] {
        std::mem::take(&mut self.0)
    }
    /// The rest of the message, 1 to `max` bytes of it.
    fn sized(&mut self, max: usize, what: &'static str) -> Result<Vec<u8>, Error> {
        let bytes = self.rest();
        if !(1..=max).contains(&bytes.len()) {
            return Err(Error::InvalidValue(what));
        }
        Ok(bytes.to_vec())
    }
    /// Refuses bytes left over once the fields are read.
    fn finish(&self) -> Result<(), Error> {
        if self.0.is_empty() { Ok(()) } else { Err(Error::TrailingBytes) }
    }
    fn u8(&mut self) -> Result<u8, Error> {
        Ok(self.take(1)?[0])
    }
    fn bool(&mut self) -> Result<bool, Error> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(Error::InvalidValue("flag")),
        }
    }
    fn u16(&mut self) -> Result<u16, Error> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().expect("two bytes")))
    }
    fn u32(&mut self) -> Result<u32, Error> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().expect("four bytes")))
    }
    fn u64(&mut self) -> Result<u64, Error> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().expect("eight bytes")))
    }
    fn rect(&mut self) -> Result<Rect, Error> {
        let rect = Rect { x: self.u16()?, y: self.u16()?, w: self.u16()?, h: self.u16()? };
        if rect.w == 0 || rect.h == 0 {
            return Err(Error::InvalidValue("rectangle"));
        }
        Ok(rect)
    }
}
