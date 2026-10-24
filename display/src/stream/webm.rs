//! A recording's file: WebM (Matroska) with one AV1 video track and one Opus audio track, written as it is made, so it
//! can be saved while it grows.
//!
//! The segment has an unknown size (it is never rewritten), and the file has no `Duration` (it belongs before data
//! already sent) and, by choice, no index (`Cues`): one could follow the clusters, but players find it through a
//! `SeekHead` at the start, which would have to name where it ends up. Players that seek walk the clusters instead. Each video keyframe (or a cluster grown too long) opens a cluster; a cluster is
//! written whole, with its size, once the sound (which arrives later than the picture) has reached the next one, so
//! every block of the file follows the one before it in time. Blocks within a cluster are written in time order.
//!
//! AV1 blocks carry their temporal unit without temporal delimiters, and the track's `CodecPrivate` is the `av1C`
//! record with the stream's sequence header (Matroska's AV1 mapping); Opus blocks carry one packet each, after an
//! `OpusHead` (RFC 7845) in the track's `CodecPrivate`.

use std::collections::VecDeque;

/// Matroska's "unknown size" for the segment, which is never rewritten.
const UNKNOWN: u64 = 0x00ff_ffff_ffff_ffff;
/// A cluster is closed past this even without a new keyframe: its blocks' times are 16-bit offsets in milliseconds.
const MAX_CLUSTER_MS: u64 = 30_000;
/// Clusters held open waiting for sound; past this the oldest is written anyway (the sound stopped coming).
const MAX_OPEN: usize = 4;
const VIDEO_TRACK: u8 = 1;
const AUDIO_TRACK: u8 = 2;
/// AV1 OBU types.
const OBU_SEQUENCE_HEADER: u8 = 1;
const OBU_TEMPORAL_DELIMITER: u8 = 2;

/// The writer. Every call returns the bytes it made ready, in order, to append to the file.
pub struct Webm {
    width: u32,
    height: u32,
    /// The encoder's Opus pre-skip, in 48 kHz samples.
    pre_skip: u16,
    started: bool,
    /// The clusters not yet written, oldest first.
    open: VecDeque<Cluster>,
}

struct Cluster {
    start_ms: u64,
    blocks: Vec<Block>,
}

struct Block {
    track: u8,
    ms: u64,
    keyframe: bool,
    data: Vec<u8>,
}

impl Webm {
    pub fn new(width: u32, height: u32, pre_skip: u16) -> Webm {
        Webm { width, height, pre_skip, started: false, open: VecDeque::new() }
    }

    /// One video temporal unit at `ms`. The file starts with the first keyframe (its sequence header names the
    /// stream); anything before it is dropped.
    pub fn video(&mut self, ms: u64, keyframe: bool, unit: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        if !self.started {
            if !keyframe {
                return out;
            }
            let Some(sequence) = obus(unit).into_iter().find(|(kind, _)| *kind == OBU_SEQUENCE_HEADER) else {
                return out;
            };
            out.extend(self.header(sequence.1));
            self.started = true;
        }
        let long = self.open.back().is_some_and(|c| ms.saturating_sub(c.start_ms) >= MAX_CLUSTER_MS);
        if keyframe || long || self.open.is_empty() {
            self.open.push_back(Cluster { start_ms: ms, blocks: Vec::new() });
            while self.open.len() > MAX_OPEN {
                out.extend(self.close_oldest());
            }
        }
        let mut data = Vec::with_capacity(unit.len());
        for (_, obu) in obus(unit).into_iter().filter(|(kind, _)| *kind != OBU_TEMPORAL_DELIMITER) {
            data.extend_from_slice(obu);
        }
        if let Some(cluster) = self.open.back_mut() {
            cluster.blocks.push(Block { track: VIDEO_TRACK, ms, keyframe, data });
        }
        out
    }

    /// One Opus packet starting at `ms`, into the cluster whose time it falls in; sound before the first video
    /// keyframe, or older than every cluster still open (it belongs in one already written), is dropped. The clusters
    /// the sound has passed are written.
    pub fn audio(&mut self, ms: u64, packet: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let (Some(oldest), Some(newest)) = (self.open.front(), self.open.back()) else { return out };
        if ms < oldest.start_ms {
            return out;
        }
        // Sound far past the newest picture would overflow its cluster's 16-bit time offset: it starts a cluster.
        if ms.saturating_sub(newest.start_ms) >= MAX_CLUSTER_MS {
            self.open.push_back(Cluster { start_ms: ms, blocks: Vec::new() });
        }
        let mut at = self.open.iter().rposition(|c| c.start_ms <= ms).expect("the oldest cluster starts no later");
        // Sound catching up after a stall can fall long after the cluster it belongs in: it starts one after it.
        if ms.saturating_sub(self.open[at].start_ms) >= MAX_CLUSTER_MS {
            at += 1;
            self.open.insert(at, Cluster { start_ms: ms, blocks: Vec::new() });
        }
        self.open[at].blocks.push(Block { track: AUDIO_TRACK, ms, keyframe: true, data: packet.to_vec() });
        while self.open.len() >= 2 && ms >= self.open[1].start_ms {
            out.extend(self.close_oldest());
        }
        out
    }

    /// Ends the file: the clusters still open.
    pub fn finish(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        while !self.open.is_empty() {
            out.extend(self.close_oldest());
        }
        out
    }

    fn header(&self, sequence_header: &[u8]) -> Vec<u8> {
        let mut out = element(
            0x1A45_DFA3,
            &[
                uint(0x4286, 1),
                uint(0x42F7, 1),
                uint(0x42F2, 4),
                uint(0x42F3, 8),
                element(0x4282, b"webm"),
                uint(0x4287, 4),
                uint(0x4285, 2),
            ]
            .concat(),
        );
        // The segment: its ID and the unknown size.
        out.extend(id(0x1853_8067));
        out.extend(size(UNKNOWN));
        out.extend(element(
            0x1549_A966,
            &[uint(0x2AD7B1, 1_000_000), element(0x4D80, b"easi.run"), element(0x5741, b"easi.run")].concat(),
        ));
        // av1C: version 1, Main profile, level 4.0, main tier, 8-bit 4:2:0 (as the encoder is set up), then the
        // sequence header OBU.
        let mut av1c = vec![0x81, 0x08, 0x0C, 0x00];
        av1c.extend(sequence_header);
        let video = element(
            0xAE,
            &[
                uint(0xD7, u64::from(VIDEO_TRACK)),
                uint(0x73C5, u64::from(VIDEO_TRACK)),
                uint(0x83, 1),
                element(0x86, b"V_AV1"),
                element(0x63A2, &av1c),
                element(0xE0, &[uint(0xB0, u64::from(self.width)), uint(0xBA, u64::from(self.height))].concat()),
            ]
            .concat(),
        );
        let mut opus_head = b"OpusHead".to_vec();
        opus_head.push(1);
        opus_head.push(2);
        opus_head.extend(self.pre_skip.to_le_bytes());
        opus_head.extend(48_000u32.to_le_bytes());
        opus_head.extend(0i16.to_le_bytes());
        opus_head.push(0);
        let delay_ns = u64::from(self.pre_skip) * 1_000_000_000 / 48_000;
        let audio = element(
            0xAE,
            &[
                uint(0xD7, u64::from(AUDIO_TRACK)),
                uint(0x73C5, u64::from(AUDIO_TRACK)),
                uint(0x83, 2),
                element(0x86, b"A_OPUS"),
                element(0x63A2, &opus_head),
                uint(0x56AA, delay_ns),
                uint(0x56BB, 80_000_000),
                element(0xE1, &[element(0xB5, &48_000f64.to_be_bytes()), uint(0x9F, 2)].concat()),
            ]
            .concat(),
        );
        out.extend(element(0x1654_AE6B, &[video, audio].concat()));
        out
    }

    fn close_oldest(&mut self) -> Vec<u8> {
        let Some(mut cluster) = self.open.pop_front() else { return Vec::new() };
        if cluster.blocks.is_empty() {
            return Vec::new();
        }
        cluster.blocks.sort_by_key(|b| b.ms);
        let mut body = uint(0xE7, cluster.start_ms);
        for block in &cluster.blocks {
            let offset =
                (block.ms as i64 - cluster.start_ms as i64).clamp(i64::from(i16::MIN), i64::from(i16::MAX)) as i16;
            let mut simple = vec![0x80 | block.track];
            simple.extend(offset.to_be_bytes());
            simple.push(if block.keyframe { 0x80 } else { 0x00 });
            simple.extend(&block.data);
            body.extend(element(0xA3, &simple));
        }
        element(0x1F43_B675, &body)
    }
}

/// The OBUs of a temporal unit (low-overhead format), each with its type and its whole bytes, header included.
pub(super) fn obus(mut data: &[u8]) -> Vec<(u8, &[u8])> {
    let mut found = Vec::new();
    while let Some(&header) = data.first() {
        let kind = (header >> 3) & 0x0f;
        let extension = usize::from(header & 0x04 != 0);
        if header & 0x02 == 0 {
            // No size field: the OBU runs to the end.
            found.push((kind, data));
            break;
        }
        let mut at = 1 + extension;
        let (mut length, mut shift) = (0usize, 0);
        loop {
            let Some(&b) = data.get(at) else { return found };
            at += 1;
            length |= usize::from(b & 0x7f) << shift;
            shift += 7;
            if b & 0x80 == 0 || shift > 56 {
                break;
            }
        }
        let end = at + length;
        let Some(obu) = data.get(..end) else { return found };
        found.push((kind, obu));
        data = &data[end..];
    }
    found
}

/// An EBML element ID's bytes (IDs carry their own length marker).
fn id(value: u32) -> Vec<u8> {
    let bytes = value.to_be_bytes();
    let first = bytes.iter().position(|&b| b != 0).unwrap_or(3);
    bytes[first..].to_vec()
}

/// An EBML size in the fewest bytes that hold it (the all-ones value of each length is reserved).
fn size(value: u64) -> Vec<u8> {
    if value == UNKNOWN {
        return vec![0x01, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff];
    }
    for len in 1..=8u32 {
        if value < (1u64 << (7 * len)) - 1 {
            let marked = value | 1u64 << (7 * len);
            return marked.to_be_bytes()[8 - len as usize..].to_vec();
        }
    }
    unreachable!("an EBML size fits eight bytes")
}

fn element(element_id: u32, body: &[u8]) -> Vec<u8> {
    let mut out = id(element_id);
    out.extend(size(body.len() as u64));
    out.extend(body);
    out
}

fn uint(element_id: u32, value: u64) -> Vec<u8> {
    let bytes = value.to_be_bytes();
    let first = bytes.iter().position(|&b| b != 0).unwrap_or(7);
    element(element_id, &bytes[first..])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A temporal unit: a temporal delimiter, then a sequence header and a frame OBU (both with size fields).
    fn unit(keyframe: bool) -> Vec<u8> {
        let mut u = vec![0x12, 0x00];
        if keyframe {
            u.extend([0x0A, 0x03, 0xAA, 0xBB, 0xCC]);
        }
        u.extend([0x32, 0x02, 0x11, 0x22]);
        u
    }

    /// Reads one EBML element at `at`: its ID, the body's range, and where the next element starts.
    fn read(data: &[u8], at: usize) -> (u32, std::ops::Range<usize>, usize) {
        let id_len = data[at].leading_zeros() as usize + 1;
        let element = data[at..at + id_len].iter().fold(0u32, |v, &b| v << 8 | u32::from(b));
        let size_len = data[at + id_len].leading_zeros() as usize + 1;
        let mut length = u64::from(data[at + id_len]) & (0xff >> size_len);
        for &b in &data[at + id_len + 1..at + id_len + size_len] {
            length = length << 8 | u64::from(b);
        }
        let start = at + id_len + size_len;
        let end = if length == (1u64 << (7 * size_len)) - 1 { data.len() } else { start + length as usize };
        (element, start..end, end)
    }

    #[test]
    fn sizes_take_the_fewest_bytes_and_skip_the_reserved_values() {
        assert_eq!(size(0), vec![0x80]);
        assert_eq!(size(126), vec![0xFE]);
        assert_eq!(size(127), vec![0x40, 0x7F]);
        assert_eq!(size(16_382), vec![0x7F, 0xFE]);
        assert_eq!(size(16_383), vec![0x20, 0x3F, 0xFF]);
        assert_eq!(id(0x1A45_DFA3), vec![0x1A, 0x45, 0xDF, 0xA3]);
        assert_eq!(id(0xA3), vec![0xA3]);
    }

    #[test]
    fn the_file_starts_at_a_keyframe_and_writes_whole_clusters_in_time_order() {
        let mut webm = Webm::new(1280, 800, 312);
        assert!(webm.video(0, false, &unit(false)).is_empty(), "nothing before the first keyframe");
        assert!(webm.audio(5, &[0xfc, 1]).is_empty(), "no sound before it either");
        let mut file = webm.video(40, true, &unit(true));
        file.extend(webm.audio(60, &[0xfc, 2]));
        file.extend(webm.audio(50, &[0xfc, 3]));
        file.extend(webm.video(73, false, &unit(false)));
        file.extend(webm.video(2_040, true, &unit(true)));
        file.extend(webm.finish());

        // EBML header, then the segment with info, tracks and two clusters.
        let (ebml, _, next) = read(&file, 0);
        assert_eq!(ebml, 0x1A45_DFA3);
        let (segment, body, _) = read(&file, next);
        assert_eq!(segment, 0x1853_8067);
        let mut at = body.start;
        let mut kinds = Vec::new();
        let mut clusters = Vec::new();
        while at < body.end {
            let (element, range, next) = read(&file, at);
            kinds.push(element);
            if element == 0x1F43_B675 {
                clusters.push(range);
            }
            at = next;
        }
        assert_eq!(kinds, vec![0x1549_A966, 0x1654_AE6B, 0x1F43_B675, 0x1F43_B675]);
        // The first cluster: its time, then blocks in time order (video at 40, sound at 50 and 60, video at 73).
        let (time, range, mut at) = read(&file, clusters[0].start);
        assert_eq!(time, 0xE7);
        assert_eq!(file[range], [40]);
        let mut blocks = Vec::new();
        while at < clusters[0].end {
            let (element, range, next) = read(&file, at);
            assert_eq!(element, 0xA3);
            let block = &file[range];
            blocks.push((block[0] & 0x7f, i16::from_be_bytes([block[1], block[2]]), block[3], block[4..].to_vec()));
            at = next;
        }
        assert_eq!(
            blocks,
            vec![
                (1, 0, 0x80, vec![0x0A, 0x03, 0xAA, 0xBB, 0xCC, 0x32, 0x02, 0x11, 0x22]),
                (2, 10, 0x80, vec![0xfc, 3]),
                (2, 20, 0x80, vec![0xfc, 2]),
                (1, 33, 0x00, vec![0x32, 0x02, 0x11, 0x22]),
            ],
            "temporal delimiters are dropped; the keyframe keeps its sequence header"
        );
    }

    /// Every block's time in `file`, in file order.
    fn block_times(file: &[u8]) -> Vec<(u8, i64)> {
        let (_, _, next) = read(file, 0);
        let (_, body, _) = read(file, next);
        let mut times = Vec::new();
        let mut at = body.start;
        while at < body.end {
            let (element, range, next) = read(file, at);
            if element == 0x1F43_B675 {
                let (_, time, mut inner) = read(file, range.start);
                let base = file[time].iter().fold(0i64, |v, &b| v << 8 | i64::from(b));
                while inner < range.end {
                    let (_, block, after) = read(file, inner);
                    let block = &file[block];
                    times.push((block[0] & 0x7f, base + i64::from(i16::from_be_bytes([block[1], block[2]]))));
                    inner = after;
                }
            }
            at = next;
        }
        times
    }

    #[test]
    fn a_cluster_waits_for_the_late_sound_so_time_never_goes_back() {
        let mut webm = Webm::new(1280, 800, 312);
        let mut file = webm.video(0, true, &unit(true));
        file.extend(webm.video(1_966, false, &unit(false)));
        file.extend(webm.video(2_000, true, &unit(true)));
        assert!(webm.audio(1_580, &[0xfc, 1]).is_empty(), "the first cluster waits for the sound to reach 2000");
        file.extend(webm.video(2_033, false, &unit(false)));
        let written = webm.audio(2_000, &[0xfc, 2]);
        assert!(!written.is_empty(), "the sound reached the next cluster: the first is written");
        file.extend(written);
        file.extend(webm.finish());
        let times = block_times(&file);
        assert_eq!(times, vec![(1, 0), (2, 1_580), (1, 1_966), (1, 2_000), (2, 2_000), (1, 2_033)]);
    }

    #[test]
    fn sound_long_after_its_cluster_starts_one_of_its_own() {
        let mut webm = Webm::new(1280, 800, 312);
        let mut file = webm.video(0, true, &unit(true));
        // The encoder stalled 40 s: the next picture is far ahead of the sound.
        file.extend(webm.video(40_000, true, &unit(true)));
        file.extend(webm.audio(34_000, &[0xfc, 1]));
        file.extend(webm.audio(40_000, &[0xfc, 2]));
        file.extend(webm.finish());
        assert_eq!(block_times(&file), vec![(1, 0), (2, 34_000), (1, 40_000), (2, 40_000)]);
    }

    #[test]
    fn sound_older_than_every_open_cluster_is_dropped() {
        let mut webm = Webm::new(1280, 800, 312);
        let mut file = webm.video(1_000, true, &unit(true));
        assert!(webm.audio(900, &[0xfc, 1]).is_empty(), "sound before the first keyframe");
        file.extend(webm.video(2_000, true, &unit(true)));
        file.extend(webm.audio(2_000, &[0xfc, 2]));
        assert!(webm.audio(1_500, &[0xfc, 3]).is_empty(), "sound of a cluster already written");
        file.extend(webm.finish());
        assert_eq!(block_times(&file), vec![(1, 1_000), (1, 2_000), (2, 2_000)]);
    }

    #[test]
    fn clusters_without_sound_are_written_after_a_few() {
        let mut webm = Webm::new(1280, 800, 312);
        let mut file = webm.video(0, true, &unit(true));
        for k in 1..=MAX_OPEN as u64 {
            file.extend(webm.video(k * 2_000, true, &unit(true)));
        }
        assert!(file.windows(4).any(|w| w == [0x1F, 0x43, 0xB6, 0x75]), "the oldest went out");
    }

    #[test]
    fn the_av1_track_carries_the_sequence_header() {
        let mut webm = Webm::new(1280, 800, 312);
        let file = webm.video(0, true, &unit(true));
        let av1c = [0x81, 0x08, 0x0C, 0x00, 0x0A, 0x03, 0xAA, 0xBB, 0xCC];
        assert!(file.windows(av1c.len()).any(|w| w == av1c));
        assert!(file.windows(8).any(|w| w == b"OpusHead"));
    }
}
