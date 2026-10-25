//! AV1 (SVT-AV1, real-time preset, screen-content tools) of the output or a region of it, through the C shim in
//! `av1.c`.
//!
//! With `opus`, this module holds the display's `unsafe` code: four functions of the shim (the encoder's open, encode,
//! and close, and the heap trim), each call with the invariants stated beside it.
#![allow(unsafe_code)]

use std::ffi::{c_int, c_long, c_void};
use std::sync::{PoisonError, RwLock, RwLockWriteGuard, TryLockError};
use std::time::{Duration, Instant};

use super::yuv::Yuv;

unsafe extern "C" {
    fn display_av1_open(out: *mut *mut c_void, width: u32, height: u32, kbps: u32, fps: u32) -> c_int;
    fn display_av1_encode(
        e: *mut c_void,
        y: *const u8,
        u: *const u8,
        v: *const u8,
        y_stride: u32,
        uv_stride: u32,
        keyframe: c_int,
        out: *mut u8,
        cap: usize,
    ) -> c_long;
    fn display_av1_close(e: *mut c_void);
    fn display_trim_heap();
}

/// Hands the heap's free pages back to the system, as after releasing a video encoder: glibc would keep tens of MiB.
pub fn trim_heap() {
    // SAFETY: malloc_trim only walks the allocator's own free lists; it takes no arguments from us.
    unsafe { display_trim_heap() }
}

/// Written while an encoder starts or ends, read while one encodes: SVT-AV1 rewrites process-wide tables as an encoder
/// starts, and two viewers' encoders starting at once, or one starting while another encodes, crash the process.
static LIFECYCLE: RwLock<()> = RwLock::new(());

/// How long an encoder that starts or ends tries for the lifecycle lock before it waits in line for it.
const PATIENCE: Duration = Duration::from_millis(100);

/// Takes `lock` for writing. A writer waiting in line makes every new reader wait behind it, so a viewer's encoder
/// restarting while the recording, at its lower priority, encodes a whole frame would stall every live encode until
/// that frame is done. The writer instead tries every millisecond, which no reader waits behind, and gets in at the
/// first moment no encode runs; only past `patience` (encodes overlapping without a gap) does it wait in line, so it
/// still always starts.
fn exclusive(lock: &RwLock<()>, patience: Duration) -> RwLockWriteGuard<'_, ()> {
    let deadline = Instant::now() + patience;
    loop {
        match lock.try_write() {
            Ok(alone) => return alone,
            Err(TryLockError::Poisoned(poisoned)) => return poisoned.into_inner(),
            Err(TryLockError::WouldBlock) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(1)),
            Err(TryLockError::WouldBlock) => return lock.write().unwrap_or_else(PoisonError::into_inner),
        }
    }
}

pub struct Av1 {
    handle: *mut c_void,
    yuv: Yuv,
    out: Vec<u8>,
}

// The handle is owned by this value and used only through `&mut self`, so it is never used from two threads at once;
// SVT-AV1 allows a handle to move between threads.
unsafe impl Send for Av1 {}

impl Av1 {
    /// An encoder of `width` x `height` pictures at `bitrate_kbps` for `fps` frames a second: its rate control spends
    /// the bitrate over that many frames, whatever the frames' timing.
    pub fn new(width: u32, height: u32, bitrate_kbps: u32, fps: u8) -> Result<Av1, String> {
        let mut handle = std::ptr::null_mut();
        let _alone = exclusive(&LIFECYCLE, PATIENCE);
        // SAFETY: `handle` is a valid out pointer; the shim writes it only on success.
        let status = unsafe { display_av1_open(&mut handle, width, height, bitrate_kbps, u32::from(fps.max(1))) };
        if status != 0 || handle.is_null() {
            return Err("svt-av1: the encoder failed to start".into());
        }
        // A packet is never larger than the raw picture plus headers.
        let cap = width as usize * height as usize * 2 + 4096;
        Ok(Av1 { handle, yuv: Yuv::new(width, height), out: vec![0; cap] })
    }

    /// Encodes one XRGB8888 frame, its rows `stride` pixels apart, to a temporal unit; a keyframe carries its sequence
    /// header.
    pub fn encode(&mut self, pixels: &[u32], stride: usize, keyframe: bool) -> Result<Vec<u8>, String> {
        self.yuv.convert(pixels, stride)?;
        let planes = &self.yuv.planes;
        let (y, u, v) = (planes.y_plane.borrow(), planes.u_plane.borrow(), planes.v_plane.borrow());
        let chroma_rows = self.yuv.height.div_ceil(2);
        let (y_stride, uv_stride) = (planes.y_stride as usize, planes.u_stride as usize);
        if y.len() < y_stride * self.yuv.height
            || u.len() < uv_stride * chroma_rows
            || v.len() < planes.v_stride as usize * chroma_rows
            || planes.v_stride != planes.u_stride
        {
            return Err("svt-av1: the picture planes are smaller than their strides say".into());
        }
        let _shared = LIFECYCLE.read().unwrap_or_else(PoisonError::into_inner);
        // SAFETY: the handle is live (closed only in Drop); the three planes hold `stride * rows` bytes as checked
        // above and outlive the call, which copies them; `out` has `cap` writable bytes.
        let n = unsafe {
            display_av1_encode(
                self.handle,
                y.as_ptr(),
                u.as_ptr(),
                v.as_ptr(),
                y_stride as u32,
                uv_stride as u32,
                c_int::from(keyframe),
                self.out.as_mut_ptr(),
                self.out.len(),
            )
        };
        match n {
            n if n >= 0 => Ok(self.out[..n as usize].to_vec()),
            -2 => Err("svt-av1: a packet larger than its buffer".into()),
            _ => Err("svt-av1: encoding failed".into()),
        }
    }
}

impl Drop for Av1 {
    fn drop(&mut self) {
        let _alone = exclusive(&LIFECYCLE, PATIENCE);
        // SAFETY: the handle came from a successful open and is closed exactly once.
        unsafe { display_av1_close(self.handle) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The OBU types of a temporal unit.
    fn obu_types(data: &[u8]) -> Vec<u8> {
        crate::stream::webm::obus(data).into_iter().map(|(kind, _)| kind).collect()
    }

    /// Two encoders at different rates (a viewer's and a recording's) encode side by side: SVT-AV1 shares tables
    /// across a process, and only encoders of the same kind may run at once.
    #[test]
    fn encoders_at_different_rates_encode_side_by_side() {
        let (w, h) = (320u32, 240u32);
        let threads: Vec<_> = [2500, 10_000]
            .into_iter()
            .map(|kbps| {
                std::thread::spawn(move || {
                    let mut encoder = Av1::new(w, h, kbps, 30).unwrap();
                    for k in 0..30u32 {
                        let frame: Vec<u32> = (0..w * h).map(|i| 0xff00_0000 | ((i + k * 97) & 0xff) << 8).collect();
                        let unit = encoder.encode(&frame, w as usize, k % 10 == 0).unwrap();
                        assert_eq!(obu_types(&unit).contains(&1), k % 10 == 0 || k == 0);
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
    }

    /// A writer trying for the lock while a long encode holds it lets other encodes in meanwhile, and gets in once
    /// they are done.
    #[test]
    fn an_encoder_starting_lets_encodes_run_while_it_waits() {
        static LOCK: RwLock<()> = RwLock::new(());
        let recording = LOCK.read().unwrap();
        let writer = std::thread::spawn(|| {
            let started = Instant::now();
            drop(exclusive(&LOCK, Duration::from_secs(10)));
            started.elapsed()
        });
        std::thread::sleep(Duration::from_millis(20));
        // Tried rather than awaited, so a writer waiting in line fails the test instead of hanging it.
        let deadline = Instant::now() + Duration::from_millis(500);
        let live = loop {
            match LOCK.try_read() {
                Ok(_) => break true,
                Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(1)),
                Err(_) => break false,
            }
        };
        drop(recording);
        assert!(live, "a live encode waited behind the writer");
        assert!(writer.join().unwrap() < Duration::from_secs(5), "the writer got in once the encodes were done");
    }

    /// Encode time of a moving 1280x800 picture; run pinned to one CPU: `taskset -c 0 cargo test ... -- --ignored`.
    #[test]
    #[ignore]
    fn speed_at_1280x800() {
        let (w, h) = (1280u32, 800u32);
        let mut encoder = Av1::new(w, h, 2500, 60).unwrap();
        let mut times = Vec::new();
        let mut bytes = 0;
        // AV1_SPEED_RGB names raw 1280x800 RGB24 frames to encode instead of the synthetic ones.
        let raw = std::env::var("AV1_SPEED_RGB").ok().map(|p| std::fs::read(p).unwrap());
        for k in 0..120u32 {
            let frame: Vec<u32> = match &raw {
                Some(raw) => raw[(k as usize) * 1280 * 800 * 3..][..1280 * 800 * 3]
                    .chunks(3)
                    .map(|p| 0xff00_0000 | u32::from(p[0]) << 16 | u32::from(p[1]) << 8 | u32::from(p[2]))
                    .collect(),
                None => (0..w * h)
                    .map(|i| {
                        let (x, y) = (i % w + k * 8, i / w + k * 5);
                        0xff00_0000 | ((x / 64 * 37) & 255) << 16 | ((y / 64 * 53) & 255) << 8 | ((x ^ y) & 255)
                    })
                    .collect(),
            };
            let t = std::time::Instant::now();
            bytes += encoder.encode(&frame, w as usize, k == 0).unwrap().len();
            times.push(t.elapsed().as_micros());
        }
        times.sort_unstable();
        eprintln!("p50 {} us, p95 {} us, {} kbit/s at 60 fps", times[60], times[114], bytes * 8 * 60 / 120 / 1000);
    }

    #[test]
    fn keyframes_carry_the_sequence_header_and_inter_frames_are_small() {
        let (w, h) = (320u32, 240u32);
        let mut encoder = Av1::new(w, h, 1000, 30).unwrap();
        let frame: Vec<u32> = (0..w * h).map(|i| 0xff00_0000 | ((i % 251) * 0x010101)).collect();
        let key = encoder.encode(&frame, w as usize, true).unwrap();
        // 1: sequence header; 6: frame.
        let types = obu_types(&key);
        assert!(types.contains(&1) && types.contains(&6), "OBU types {types:?}");
        let next = encoder.encode(&frame, w as usize, false).unwrap();
        assert!(!obu_types(&next).contains(&1));
        assert!(next.len() < key.len());
        let again = encoder.encode(&frame, w as usize, true).unwrap();
        assert!(obu_types(&again).contains(&1), "a requested keyframe repeats the sequence header");
    }
}
