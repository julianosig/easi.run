//! Golden vectors: `vectors/v4.txt` is the normative byte form of protocol version 4 ([`VERSION`]). The client's
//! decoder is tested against the same file.

use display_protocol::frame::{self, FrameReader, Link};
use display_protocol::*;

const LAYOUT: CameraLayout = CameraLayout { x: 7_400, y: 6_800, side: 2_400, radius: 100 };

fn samples() -> Vec<(&'static str, Message)> {
    let rect = Rect { x: 64, y: 128, w: 640, h: 400 };
    vec![
        ("hello", Message::Hello { version: VERSION, codecs: CODEC_AV1 | CODEC_H264 }),
        (
            "config",
            Message::Config { version: VERSION, epoch: 7, width: 1280, height: 800, codec: Codec::H264, max_fps: 60 },
        ),
        // Every other value of each enumeration, so the browser and the display agree on all of them.
        (
            "config-av1",
            Message::Config { version: VERSION, epoch: 8, width: 1280, height: 800, codec: Codec::Av1, max_fps: 30 },
        ),
        (
            "config-tiles",
            Message::Config { version: VERSION, epoch: 9, width: 1280, height: 800, codec: Codec::None, max_fps: 60 },
        ),
        (
            "video",
            Message::Video {
                epoch: 7,
                revision: 1001,
                rect,
                keyframe: true,
                timestamp_us: 123_456_789,
                payload: vec![0, 0, 0, 1, 0x67],
            },
        ),
        ("copied", Message::Copied { lease_epoch: 3, text: "ção".into() }),
        (
            "file-offer",
            Message::FileOffer { lease_epoch: 3, file: 7, size: Some(5_000_000_000), name: "relatório.pdf".into() },
        ),
        ("file-offer-unsized", Message::FileOffer { lease_epoch: 3, file: 8, size: None, name: "a".into() }),
        ("file-link", Message::FileLink { lease_epoch: 3, file: 7, ticket: std::array::from_fn(|i| i as u8) }),
        ("file-data", Message::FileData { lease_epoch: 3, file: 7, offset: 65_536, data: vec![1, 2, 3] }),
        ("file-end", Message::FileEnd { lease_epoch: 3, file: 7, size: 5_000_000_000 }),
        ("file-gone", Message::FileGone { lease_epoch: 3, file: 7 }),
        ("file-request", Message::FileRequest { lease_epoch: 3, request: 9, multiple: true }),
        ("file-request-end", Message::FileRequestEnd { lease_epoch: 3, request: 9 }),
        ("upload-ack", Message::UploadAck { lease_epoch: 3, upload: 4, received: 61_440 }),
        ("upload-done", Message::UploadDone { lease_epoch: 3, upload: 4, stored: true }),
        // One 20 ms CELT fullband stereo Opus packet header (TOC 0xfc) and two bytes of payload.
        ("audio", Message::Audio { timestamp_us: 123_476_789, payload: vec![0xfc, 0xff, 0xfe] }),
        (
            "tiles",
            Message::Tiles {
                epoch: 7,
                revision: 1002,
                rects: vec![Rect { x: 0, y: 0, w: 64, h: 64 }, Rect { x: 64, y: 0, w: 2, h: 1 }],
                raw_len: 11,
                data: vec![0x63, 0x54, 0x70, 0x48, 0x60, 0x60, 0x64, 0x62, 0x66, 0x61, 0x65, 0x03, 0x00],
            },
        ),
        (
            "cursor",
            Message::Cursor { epoch: 7, revision: 1003, x: 300, y: 200, shape: 1, hot_x: 0, hot_y: 0, image: vec![] },
        ),
        ("keyframe-request", Message::KeyframeRequest { epoch: 7 }),
        ("media-ack", Message::MediaAck { revision: 1002 }),
        ("udp-request", Message::UdpRequest),
        (
            "udp-offer",
            Message::UdpOffer {
                generation: 2,
                cert_sha256: std::array::from_fn(|i| 0xa0 + i as u8),
                url: "https://203.0.113.7:9443/media?t=AAEC".into(),
            },
        ),
        ("media-moved", Message::MediaMoved { generation: 2 }),
        ("media-fallback", Message::MediaFallback { generation: 2 }),
        (
            "server-stats",
            Message::ServerStats {
                server_time_us: 5_000_000,
                frames: 60,
                bytes: 250_000,
                encode_us_p50: 4_000,
                encode_us_p95: 9_000,
            },
        ),
        ("client-stats", Message::ClientStats { presented: 59, decode_us_p50: 1_500, hardware: true }),
        ("time-ping", Message::TimePing { client_time_us: 42 }),
        ("time-pong", Message::TimePong { client_time_us: 42, server_time_us: 99 }),
        ("lease", Message::Lease { lease_epoch: 3, holder: LeaseHolder::You }),
        ("lease-agent", Message::Lease { lease_epoch: 4, holder: LeaseHolder::Agent }),
        ("lease-other", Message::Lease { lease_epoch: 5, holder: LeaseHolder::Other }),
        ("pointer", Message::Pointer { lease_epoch: 3, kind: PointerKind::Down, button: 0, x: 10, y: 20 }),
        ("pointer-move", Message::Pointer { lease_epoch: 3, kind: PointerKind::Move, button: 0, x: 11, y: 21 }),
        ("pointer-up", Message::Pointer { lease_epoch: 3, kind: PointerKind::Up, button: 2, x: 12, y: 22 }),
        ("wheel", Message::Wheel { lease_epoch: 3, x: 10, y: 20, dx: 0, dy: -120 }),
        ("key", Message::Key { lease_epoch: 3, kind: KeyKind::Down, code: 30, text: String::new() }),
        ("key-up", Message::Key { lease_epoch: 3, kind: KeyKind::Up, code: 30, text: String::new() }),
        ("key-text", Message::Key { lease_epoch: 3, kind: KeyKind::Text, code: 0, text: "ção".into() }),
        ("clipboard", Message::Clipboard { lease_epoch: 3, text: "hello".into() }),
        ("file-read", Message::FileRead { lease_epoch: 3, file: 7, offset: 65_536 }),
        ("file-cancel", Message::FileCancel { lease_epoch: 3, file: 7 }),
        ("upload-start", Message::UploadStart { lease_epoch: 3, upload: 4, size: 70_000, name: "foto.png".into() }),
        ("upload-data", Message::UploadData { lease_epoch: 3, upload: 4, offset: 61_440, data: vec![9, 8, 7] }),
        ("file-choose", Message::FileChoose { lease_epoch: 3, request: 9, uploads: vec![4, 5] }),
        ("file-drop", Message::FileDrop { lease_epoch: 3, x: 640, y: 400, uploads: vec![4] }),
        ("file-drag", Message::FileDrag { lease_epoch: 3, x: 640, y: 400, types: vec!["image/png".into(), "".into()] }),
        ("file-drag-end", Message::FileDragEnd { lease_epoch: 3 }),
        (
            "record-start",
            Message::RecordStart {
                lease_epoch: 3,
                recording: 2,
                layout: LAYOUT,
                camera: true,
                microphone: true,
                page_sound: false,
            },
        ),
        (
            "record-camera",
            Message::RecordCamera {
                lease_epoch: 3,
                recording: 2,
                timestamp_us: 123_456_789,
                jpeg: vec![0xff, 0xd8, 0xff],
            },
        ),
        (
            "record-microphone",
            Message::RecordMicrophone {
                lease_epoch: 3,
                recording: 2,
                timestamp_us: 123_476_789,
                payload: vec![0xfc, 0xff, 0xfe],
            },
        ),
        ("record-stop", Message::RecordStop { lease_epoch: 3, recording: 2 }),
        ("record-layout", Message::RecordLayout { lease_epoch: 3, recording: 2, layout: LAYOUT }),
        (
            "record-state",
            Message::RecordState {
                lease_epoch: 3,
                recording: 2,
                phase: RecordPhase::Recording,
                file: 9,
                seconds: 75,
                bytes: 5_000_000_000,
            },
        ),
        (
            "record-state-ended",
            Message::RecordState {
                lease_epoch: 3,
                recording: 2,
                phase: RecordPhase::Ended,
                file: 9,
                seconds: 80,
                bytes: 6,
            },
        ),
        (
            "record-state-failed",
            Message::RecordState {
                lease_epoch: 3,
                recording: 2,
                phase: RecordPhase::Failed,
                file: 0,
                seconds: 3,
                bytes: 0,
            },
        ),
    ]
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn golden_vectors_match() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/vectors/v4.txt");
    let expected: String = samples().iter().map(|(name, m)| format!("{name} {}\n", hex(&m.encode()))).collect();
    if std::env::var_os("WRITE_VECTORS").is_some() {
        std::fs::write(path, &expected).unwrap();
    }
    assert_eq!(std::fs::read_to_string(path).unwrap(), expected);
    for (name, message) in samples() {
        assert_eq!(Message::decode(&message.encode(), message.direction()).unwrap(), message);
        assert!(message.size_hint() >= message.encode().len(), "{name}: the size hint is short");
    }
}

#[test]
fn refuses_malformed_messages() {
    let c2s = Direction::ClientToServer;
    assert_eq!(Message::decode(&[], c2s), Err(Error::Empty));
    assert_eq!(Message::decode(&[0x7f], c2s), Err(Error::UnknownKind(0x7f)));
    assert_eq!(Message::decode(&[0x06, 1, 0, 0], c2s), Err(Error::Truncated));
    assert_eq!(Message::decode(&[0x06, 1, 0, 0, 0, 9], c2s), Err(Error::TrailingBytes));
    // A server message arriving from a client is refused.
    let config = samples()[1].1.encode();
    assert_eq!(Message::decode(&config, c2s), Err(Error::InvalidValue("direction")));
    // Invalid UTF-8, an empty text key, a text on a physical key, a bad button, and an oversized paste.
    assert!(Message::decode(&[0x13, 3, 0, 0, 0, 0xff], c2s).is_err());
    assert!(Message::decode(&[0x12, 3, 0, 0, 0, 2, 0, 0], c2s).is_err());
    assert!(Message::decode(&[0x12, 3, 0, 0, 0, 0, 30, 0, b'a'], c2s).is_err());
    // A physical key needs a code, and composed text has none.
    assert!(Message::decode(&[0x12, 3, 0, 0, 0, 0, 0, 0], c2s).is_err());
    assert!(Message::decode(&[0x12, 3, 0, 0, 0, 2, 30, 0, b'a'], c2s).is_err());
    assert!(Message::decode(&[0x10, 3, 0, 0, 0, 1, 9, 0, 0, 0, 0], c2s).is_err());
    let mut paste = vec![0x13, 3, 0, 0, 0];
    paste.extend(std::iter::repeat_n(b'a', MAX_CLIPBOARD + 1));
    assert!(Message::decode(&paste, c2s).is_err());
    // A hello of another version and one with an unknown codec bit; a zero-size
    // configuration.
    for version in [VERSION - 1, VERSION + 1] {
        let [lo, hi] = u16::to_le_bytes(version);
        assert_eq!(Message::decode(&[0x01, lo, hi, 3], c2s), Err(Error::InvalidValue("version")));
    }
    let [lo, hi] = VERSION.to_le_bytes();
    assert_eq!(Message::decode(&[0x01, lo, hi, 0x04], c2s), Err(Error::InvalidValue("codec set")));
    let s2c = Direction::ServerToClient;
    assert!(Message::decode(&[0x02, lo, hi, 7, 0, 0, 0, 0, 0, 0x20, 3, 2, 60], s2c).is_err());
    // A tile wider than MAX_TILE_SIDE (the rectangle's width sits at bytes 19-20 of `tiles`).
    let mut wide = samples().iter().find(|(name, _)| *name == "tiles").unwrap().1.encode();
    wide[19..21].copy_from_slice(&(MAX_TILE_SIDE + 1).to_le_bytes());
    assert_eq!(Message::decode(&wide, s2c), Err(Error::InvalidValue("tile size")));
    // A records length shorter than all solid records or longer than all RGB ones (it follows the two rectangles).
    let tiles = samples().iter().find(|(name, _)| *name == "tiles").unwrap().1.encode();
    for raw_len in [7u32, 64 * 64 * 3 + 1 + 7 + 1] {
        let mut bad = tiles.clone();
        bad[31..35].copy_from_slice(&raw_len.to_le_bytes());
        assert_eq!(Message::decode(&bad, s2c), Err(Error::InvalidValue("tile records length")));
    }
    // An empty or over-long audio packet, and audio from a client.
    assert_eq!(Message::decode(&[0x0c, 0, 0, 0, 0, 0, 0, 0, 0], s2c), Err(Error::InvalidValue("audio packet size")));
    let mut long = vec![0x0c, 0, 0, 0, 0, 0, 0, 0, 0];
    long.extend(std::iter::repeat_n(0xfc, MAX_AUDIO_PACKET + 1));
    assert_eq!(Message::decode(&long, s2c), Err(Error::InvalidValue("audio packet size")));
    let audio = samples().iter().find(|(name, _)| *name == "audio").unwrap().1.encode();
    assert_eq!(Message::decode(&audio, c2s), Err(Error::InvalidValue("direction")));
    // An upload without a name or with an empty piece; upload lists not of whole ids, too long, or an empty drop.
    assert_eq!(
        Message::decode(&[0x15, 3, 0, 0, 0, 4, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0], c2s),
        Err(Error::InvalidValue("file name"))
    );
    let empty_piece = [0x16, 3, 0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    assert_eq!(Message::decode(&empty_piece, c2s), Err(Error::InvalidValue("upload piece size")));
    assert_eq!(Message::decode(&[0x17, 3, 0, 0, 0, 9, 0, 0, 0, 4, 0], c2s), Err(Error::InvalidValue("upload list")));
    let mut many = vec![0x17, 3, 0, 0, 0, 9, 0, 0, 0];
    many.extend(std::iter::repeat_n(0, 4 * (MAX_CHOSEN + 1)));
    assert_eq!(Message::decode(&many, c2s), Err(Error::InvalidValue("upload list")));
    assert_eq!(Message::decode(&[0x18, 3, 0, 0, 0, 0, 0, 0, 0], c2s), Err(Error::InvalidValue("upload list")));
    // A drag of no files, a type outside the lowercase type/subtype form, and a type longer than its bytes.
    assert_eq!(Message::decode(&[0x21, 3, 0, 0, 0, 0, 0, 0, 0, 0], c2s), Err(Error::InvalidValue("drag types")));
    for bad in [&b"Image/png"[..], b"image", b"image/", b"/png", b"image/png/x", b"text/plain; charset=utf-8"] {
        let mut drag = vec![0x21, 3, 0, 0, 0, 0, 0, 0, 0, 1, bad.len() as u8];
        drag.extend_from_slice(bad);
        assert_eq!(Message::decode(&drag, c2s), Err(Error::InvalidValue("media type")), "{bad:?}");
    }
    assert_eq!(Message::decode(&[0x21, 3, 0, 0, 0, 0, 0, 0, 0, 1, 3, b'a', b'/'], c2s), Err(Error::Truncated));
    // A file offer without a name or with a name over its bound; a file piece empty or over its bound.
    let offer = [0x0e, 3, 0, 0, 0, 7, 0, 0, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0];
    // A size said to be unknown must be zero.
    assert_eq!(
        Message::decode(&[0x0e, 3, 0, 0, 0, 7, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, b'a'], s2c),
        Err(Error::InvalidValue("file size"))
    );
    assert_eq!(Message::decode(&offer, s2c), Err(Error::InvalidValue("file name")));
    let mut named = offer.to_vec();
    named.extend(std::iter::repeat_n(b'a', MAX_FILE_NAME + 1));
    assert_eq!(Message::decode(&named, s2c), Err(Error::TooLarge));
    let piece = [0x0f, 3, 0, 0, 0, 7, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    assert_eq!(Message::decode(&piece, s2c), Err(Error::InvalidValue("file piece size")));
    let mut big = piece.to_vec();
    big.extend(std::iter::repeat_n(0, FILE_CHUNK + 1));
    assert_eq!(Message::decode(&big, s2c), Err(Error::InvalidValue("file piece size")));
    // A download ticket of another length.
    let mut link = vec![0x20, 3, 0, 0, 0, 7, 0, 0, 0];
    link.extend([0; FILE_TICKET - 1]);
    assert_eq!(Message::decode(&link, s2c), Err(Error::Truncated));
    link.extend([0, 0]);
    assert_eq!(Message::decode(&link, s2c), Err(Error::TrailingBytes));
    // Copied text over its bound, not UTF-8, or from a client.
    let mut copied = vec![0x0d, 3, 0, 0, 0];
    copied.extend(std::iter::repeat_n(b'a', MAX_COPIED + 1));
    assert_eq!(Message::decode(&copied, s2c), Err(Error::TooLarge));
    assert_eq!(Message::decode(&[0x0d, 3, 0, 0, 0, 0xff], s2c), Err(Error::InvalidValue("utf-8")));
    assert_eq!(Message::decode(&[0x0d, 3, 0, 0, 0, b'a'], c2s), Err(Error::InvalidValue("direction")));
    // A cursor shape the protocol does not number, a named shape with an image, and a custom one without.
    let cursor = |shape: u8, image: &[u8]| {
        let mut m = vec![0x05, 7, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, shape, 0, 0, 0, 0];
        m.extend_from_slice(image);
        m
    };
    assert!(Message::decode(&cursor(0, &[]), s2c).is_ok() && Message::decode(&cursor(34, &[]), s2c).is_ok());
    assert!(Message::decode(&cursor(255, b"qoif"), s2c).is_ok());
    for bad in [cursor(35, &[]), cursor(254, &[]), cursor(1, b"qoif"), cursor(255, &[])] {
        assert_eq!(Message::decode(&bad, s2c), Err(Error::InvalidValue("cursor shape")), "{bad:?}");
    }
    // A configuration of another version.
    let [lo, hi] = (VERSION + 1).to_le_bytes();
    assert_eq!(
        Message::decode(&[0x02, lo, hi, 7, 0, 0, 0, 0, 5, 0x20, 3, 2, 60], s2c),
        Err(Error::InvalidValue("configuration"))
    );
    // Recording: a camera position past the screen, a side or radius out of range, an unknown flag; an empty
    // or oversized picture; an empty or oversized packet; a state from a client.
    let start = [0x23, 3, 0, 0, 0, 2, 0, 0, 0, 0x10, 0x27, 0, 0, 0xa0, 0x0f, 100, 3];
    assert!(Message::decode(&start, c2s).is_ok(), "x at the right edge, the largest side, a circle");
    let invalid = |at: usize, value: u8, why: &'static str| {
        let mut bad = start;
        bad[at] = value;
        assert_eq!(Message::decode(&bad, c2s), Err(Error::InvalidValue(why)));
    };
    invalid(9, 0x11, "camera position");
    invalid(13, 0xa1, "camera side");
    invalid(14, 0x03, "camera side");
    invalid(15, 101, "camera radius");
    invalid(16, 8, "recording flags");
    let mut smallest = start;
    smallest[13..15].copy_from_slice(&CAMERA_SIDE_MIN.to_le_bytes());
    assert!(Message::decode(&smallest, c2s).is_ok(), "the smallest side");
    let layout = [0x28, 3, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0x11, 0x27, 0x60, 0x09, 0];
    assert_eq!(Message::decode(&layout, c2s), Err(Error::InvalidValue("camera position")));
    let header = [0x24, 3, 0, 0, 0, 2, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0];
    assert_eq!(Message::decode(&header, c2s), Err(Error::InvalidValue("camera picture size")));
    let mut picture = header.to_vec();
    picture.extend(std::iter::repeat_n(0, MAX_RECORD_PICTURE + 1));
    assert_eq!(Message::decode(&picture, c2s), Err(Error::InvalidValue("camera picture size")));
    let mut packet = vec![0x25, 3, 0, 0, 0, 2, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0];
    assert_eq!(Message::decode(&packet, c2s), Err(Error::InvalidValue("microphone packet size")));
    packet.extend(std::iter::repeat_n(0, MAX_AUDIO_PACKET + 1));
    assert_eq!(Message::decode(&packet, c2s), Err(Error::InvalidValue("microphone packet size")));
    let state = samples().iter().find(|(name, _)| *name == "record-state").unwrap().1.encode();
    assert_eq!(Message::decode(&state, c2s), Err(Error::InvalidValue("direction")));
    let mut phase = state.clone();
    phase[9] = 3;
    assert_eq!(Message::decode(&phase, s2c), Err(Error::InvalidValue("recording phase")));
}

#[test]
fn frames_are_bounded_and_split() {
    let mut reader = FrameReader::new(Direction::ClientToServer);
    let a = Link::Attach { viewer: 9 }.encode();
    let b = Link::Grant { lease_epoch: 4, holder: true }.encode();
    let mut stream = a.clone();
    stream.extend_from_slice(&b);
    reader.push(&stream[..3]).unwrap();
    assert_eq!(reader.next_body().unwrap(), None);
    reader.push(&stream[3..]).unwrap();
    let first = reader.next_body().unwrap().unwrap();
    assert_eq!(Link::decode(&first, Direction::ClientToServer).unwrap(), Link::Attach { viewer: 9 });
    let second = reader.next_body().unwrap().unwrap();
    assert_eq!(Link::decode(&second, Direction::ClientToServer).unwrap(), Link::Grant { lease_epoch: 4, holder: true });
    let settings = Link::Settings(frame::Settings { codec: frame::CodecChoice::H264, bitrate_kbps: 2500, max_fps: 60 });
    assert_eq!(Link::decode(&settings.encode()[4..], Direction::ClientToServer).unwrap(), settings);
    let bad = Link::Settings(frame::Settings { codec: frame::CodecChoice::H264, bitrate_kbps: 1, max_fps: 60 });
    assert!(Link::decode(&bad.encode()[4..], Direction::ClientToServer).is_err());
    // A page to open goes only to the display, an https address; the answer only comes back.
    let open = Link::OpenPage { request: 7, lease_epoch: 3, url: "https://sound.play/".into() };
    assert_eq!(Link::decode(&open.encode()[4..], Direction::ClientToServer).unwrap(), open);
    assert!(Link::decode(&open.encode()[4..], Direction::ServerToClient).is_err());
    let mut plain = open.encode()[4..].to_vec();
    plain.truncate(9);
    plain.extend_from_slice(b"http://sound.play/");
    assert_eq!(Link::decode(&plain, Direction::ClientToServer), Err(Error::InvalidValue("page address")));
    let opened = Link::PageOpened { request: 7, opened: true };
    assert_eq!(Link::decode(&opened.encode()[4..], Direction::ServerToClient).unwrap(), opened);
    assert!(Link::decode(&opened.encode()[4..], Direction::ClientToServer).is_err());
    // The simulator's lost media goes only to the display.
    let discarded = Link::MediaDiscarded { revision: 1002 };
    assert_eq!(Link::decode(&discarded.encode()[4..], Direction::ClientToServer).unwrap(), discarded);
    assert!(Link::decode(&discarded.encode()[4..], Direction::ServerToClient).is_err());
    // A media acknowledgement comes only from a client, as a relayed message.
    let ack = Message::MediaAck { revision: 7 }.encode();
    assert_eq!(Message::decode(&ack, Direction::ServerToClient), Err(Error::InvalidValue("direction")));
    assert_eq!(Message::decode(&ack[..8], Direction::ClientToServer), Err(Error::Truncated));
    // A path change goes only to the display.
    let changed = Link::PathChanged;
    assert_eq!(Link::decode(&changed.encode()[4..], Direction::ClientToServer).unwrap(), changed);
    assert!(Link::decode(&changed.encode()[4..], Direction::ServerToClient).is_err());
    // An offer names an https address of printable ASCII, after a whole certificate hash.
    let offer = |url: &str| {
        let mut bytes = vec![0x2b, 2, 0, 0, 0];
        bytes.extend([7; 32]);
        bytes.extend(url.as_bytes());
        Message::decode(&bytes, Direction::ServerToClient)
    };
    assert!(offer("https://host:9443/media?t=x").is_ok());
    for bad in ["http://host:9443/media", "https://host/a b", ""] {
        assert_eq!(offer(bad), Err(Error::InvalidValue("udp address")), "{bad:?}");
    }
    let long = format!("https://{}", "a".repeat(MAX_UDP_URL));
    assert_eq!(offer(&long), Err(Error::TooLarge));
    assert_eq!(Message::decode(&[0x2b, 2, 0, 0, 0, 7], Direction::ServerToClient), Err(Error::Truncated));
    // The display never accepts link control from its own direction.
    assert!(Link::decode(&first, Direction::ServerToClient).is_err());
    let mut huge = FrameReader::new(Direction::ClientToServer);
    assert_eq!(huge.push(&(u32::MAX).to_le_bytes()), Err(Error::TooLarge));
    // A short valid frame followed by an endless stream never grows the buffer past one frame and one chunk.
    let mut flood = FrameReader::new(Direction::ClientToServer);
    let mut big = Link::Attach { viewer: 1 }.encode();
    big.extend_from_slice(&((MAX_CLIENT_MESSAGE as u32).to_le_bytes()));
    flood.push(&big).unwrap();
    let mut refused = false;
    for _ in 0..64 {
        if flood.push(&vec![0u8; frame::MAX_CHUNK]).is_err() {
            refused = true;
            break;
        }
    }
    assert!(refused);
    assert!(flood.push(&vec![0u8; frame::MAX_CHUNK + 1]).is_err());
}

#[test]
fn a_video_message_of_the_largest_payload_is_the_largest_server_message() {
    let video = |len: usize| Message::Video {
        epoch: 1,
        revision: 1,
        rect: Rect { x: 0, y: 0, w: 64, h: 64 },
        keyframe: true,
        timestamp_us: 0,
        payload: vec![0; len],
    };
    let largest = video(MAX_VIDEO_PAYLOAD).encode();
    assert_eq!(largest.len(), MAX_SERVER_MESSAGE);
    assert!(Message::decode(&largest, Direction::ServerToClient).is_ok());
    let over = video(MAX_VIDEO_PAYLOAD + 1).encode();
    assert_eq!(Message::decode(&over, Direction::ServerToClient), Err(Error::TooLarge));
}

#[test]
fn only_the_lease_holders_messages_carry_a_lease_epoch() {
    let mut fenced = Vec::new();
    for (name, message) in samples() {
        if let Some(epoch) = message.lease_epoch() {
            assert_eq!(message.encode()[1..5], epoch.to_le_bytes(), "{name}: the epoch follows the kind");
            fenced.push(name);
        }
    }
    let unfenced: Vec<&str> =
        samples().into_iter().map(|(name, _)| name).filter(|name| !fenced.contains(name)).collect();
    assert_eq!(
        unfenced,
        [
            "hello",
            "config",
            "config-av1",
            "config-tiles",
            "video",
            "audio",
            "tiles",
            "cursor",
            "keyframe-request",
            "media-ack",
            "udp-request",
            "udp-offer",
            "media-moved",
            "media-fallback",
            "server-stats",
            "client-stats",
            "time-ping",
            "time-pong",
            "lease",
            "lease-agent",
            "lease-other"
        ]
    );
}

#[test]
fn link_records_of_another_length_are_refused() {
    let c2s = Direction::ClientToServer;
    let grant = Link::Grant { lease_epoch: 4, holder: true }.encode()[4..].to_vec();
    assert_eq!(Link::decode(&grant[..grant.len() - 1], c2s), Err(Error::Truncated));
    assert_eq!(Link::decode(&[&grant[..], &[0]].concat(), c2s), Err(Error::TrailingBytes));
    let mut flag = grant.clone();
    *flag.last_mut().unwrap() = 2;
    assert_eq!(Link::decode(&flag, c2s), Err(Error::InvalidValue("flag")));
    let attach = Link::Attach { viewer: 9 }.encode()[4..].to_vec();
    assert_eq!(Link::decode(&attach[..8], c2s), Err(Error::Truncated));
    assert_eq!(Link::decode(&[&attach[..], &[0]].concat(), c2s), Err(Error::TrailingBytes));
    let opened = Link::PageOpened { request: 7, opened: false }.encode()[4..].to_vec();
    assert_eq!(Link::decode(&[&opened[..], &[0]].concat(), Direction::ServerToClient), Err(Error::TrailingBytes));
    assert_eq!(Link::decode(&[], c2s), Err(Error::Empty));
}

#[test]
fn file_uris_encode_everything_but_unreserved_bytes_and_slashes() {
    assert_eq!(
        display_protocol::file_uri("/uploads/1/relatório final.pdf"),
        "file:///uploads/1/relat%C3%B3rio%20final.pdf"
    );
    assert_eq!(display_protocol::file_uri("/uploads/2/a#b?c%"), "file:///uploads/2/a%23b%3Fc%25");
}
