//! `wl_shm` without mapping client memory.
//!
//! Clients hand the compositor file descriptors to mutable shared memory. This implementation never maps them: it
//! copies pixels out with `pread` (and writes capture results with `pwrite`) into and from compositor-owned memory.
//! A client that truncates its pool therefore produces a short read and is disconnected, instead of a `SIGBUS` in
//! the compositor, and a client cannot exhaust the compositor's address space with mappings. Only memory-backed
//! descriptors (tmpfs: memfd, `/dev/shm`; or hugetlbfs) are accepted, so a read can never block on a slow or remote
//! file system.

use std::cell::RefCell;
use std::fs::File;
use std::io::{IoSlice, IoSliceMut};
use std::ops::Range;
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::fs::FileExt;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::Ordering;

use smithay::wayland::{Dispatch2, GlobalDispatch2};
use wayland_server::protocol::{
    wl_buffer::{self, WlBuffer},
    wl_shm::{self, WlShm},
    wl_shm_pool::{self, WlShmPool},
};
use wayland_server::{Client, DataInit, DisplayHandle, New, Resource, WEnum, backend::ClientId};

use crate::limits;
use crate::state::{Accounting, ClientState, State};

/// Rows one `preadv` or `pwritev` moves. Measured on tmpfs, a call's cost grows faster than the segments it carries
/// past a few dozen: reads (a row and the gap after it, two segments) are fastest at 32 to 64 rows, writes at 8.
const READ_BATCH_ROWS: usize = 64;
const WRITE_BATCH_ROWS: usize = 8;
/// The most bytes between rows that one `preadv` reads and drops, in all and between two rows. Measured, reading up to
/// 5 KiB between rows (any rectangle of a 1280-pixel-wide buffer) halves the time; from 7.5 KiB on it can cost more
/// than the calls it saves.
const GAP_SINK_BYTES: usize = 256 * 1024;
const MAX_GAP_BYTES: usize = 6 * 1024;

thread_local! {
    /// Where `preadv` drops the bytes between the rows of a rectangle.
    static GAP_SINK: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

const TMPFS_MAGIC: u64 = 0x0102_1994;
const HUGETLBFS_MAGIC: u64 = 0x9584_58f6;

/// User data of the `wl_shm` global.
pub struct ShmGlobal;

/// One client pool: its descriptor and its declared size.
pub struct Pool {
    fd: File,
    size: Mutex<usize>,
    accounting: Arc<Accounting>,
}

impl Pool {
    fn new(fd: OwnedFd, size: usize, accounting: Arc<Accounting>) -> Arc<Pool> {
        accounting.pool_fds.fetch_add(1, Ordering::Relaxed);
        Arc::new(Pool { fd: File::from(fd), size: Mutex::new(size), accounting })
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        self.accounting.pool_fds.fetch_sub(1, Ordering::Relaxed);
    }
}

/// One buffer: a rectangle of a pool in a supported format.
pub struct ShmBuffer {
    pool: Arc<Pool>,
    pub offset: usize,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub opaque: bool,
}

/// A pool read or write that came up short: the client truncated or replaced its memory.
#[derive(Debug)]
pub struct PoolIoError;

impl ShmBuffer {
    /// Copies rows `y..y+h`, columns `x..x+w` of the buffer into `dst` (row stride `dst_width` pixels, same origin).
    pub fn read_rect(
        &self,
        x: u32,
        y: u32,
        w: u32,
        h: u32,
        dst: &mut [u32],
        dst_width: usize,
    ) -> Result<(), PoolIoError> {
        debug_assert!(x + w <= self.width && y + h <= self.height);
        // Whole rows of a tightly packed buffer into a destination of the same width are one contiguous range.
        if x == 0 && w == self.width && self.stride == w * 4 && dst_width == w as usize {
            let src = self.offset + y as usize * self.stride as usize;
            let start = y as usize * dst_width;
            let bytes: &mut [u8] = bytemuck::cast_slice_mut(&mut dst[start..start + (w * h) as usize]);
            return self.pool.fd.read_exact_at(bytes, src as u64).map_err(|_| PoolIoError);
        }
        // Otherwise several rows per `preadv`, the bytes between them read into a sink and dropped, while that is
        // cheaper than a call per row.
        let gap = self.stride as usize - w as usize * 4;
        let batch = match GAP_SINK_BYTES.checked_div(gap) {
            _ if gap > MAX_GAP_BYTES => 1,
            Some(rows) => READ_BATCH_ROWS.min(1 + rows),
            None => READ_BATCH_ROWS,
        };
        GAP_SINK.with_borrow_mut(|sink| {
            sink.resize(gap * (batch - 1), 0);
            let mut row = y as usize;
            while row < (y + h) as usize {
                let rows = row..(row + batch).min((y + h) as usize);
                row = rows.end;
                self.read_rows(rows, x as usize..(x + w) as usize, dst, dst_width, sink)?;
            }
            Ok(())
        })
    }

    /// Reads `rows` of the buffer, `columns` of each: several in one `preadv` with the `stride - 4w` bytes between two
    /// rows going to `sink`, one with a `pread`. A short `preadv` is retried row by row, which tells a truncated pool
    /// from a read the kernel split.
    fn read_rows(
        &self,
        rows: Range<usize>,
        columns: Range<usize>,
        dst: &mut [u32],
        dst_width: usize,
        sink: &mut [u8],
    ) -> Result<(), PoolIoError> {
        let (stride, w, count) = (self.stride as usize, columns.len(), rows.len());
        let gap = stride - w * 4;
        let src = self.offset + rows.start * stride + columns.start * 4;
        let start = rows.start * dst_width + columns.start;
        if count > 1 {
            let mut gaps = sink.chunks_exact_mut(gap.max(1));
            let mut slices = Vec::with_capacity(2 * count);
            for (i, line) in dst[start..].chunks_mut(dst_width).take(count).enumerate() {
                if i > 0 && gap > 0 {
                    slices.push(IoSliceMut::new(gaps.next().expect("the sink holds a gap per row")));
                }
                slices.push(IoSliceMut::new(bytemuck::cast_slice_mut(&mut line[..w])));
            }
            let total = count * w * 4 + (count - 1) * gap;
            if rustix::io::preadv(&self.pool.fd, &mut slices, src as u64) == Ok(total) {
                return Ok(());
            }
        }
        for (i, line) in dst[start..].chunks_mut(dst_width).take(count).enumerate() {
            self.pool
                .fd
                .read_exact_at(bytemuck::cast_slice_mut(&mut line[..w]), (src + i * stride) as u64)
                .map_err(|_| PoolIoError)?;
        }
        Ok(())
    }

    /// Writes the whole buffer from `rows`, top to bottom, each `width` pixels; it fails on a row of another length or
    /// too few rows. A tightly packed buffer takes several rows per `pwritev`, a padded one a row per `pwrite`.
    pub fn write_rows<'a>(&self, rows: impl IntoIterator<Item = &'a [u32]>) -> Result<(), PoolIoError> {
        let (width, height, stride) = (self.width as usize, self.height as usize, self.stride as usize);
        let batch = if stride == width * 4 { WRITE_BATCH_ROWS } else { 1 };
        let mut rows = rows.into_iter();
        let mut lines = Vec::with_capacity(batch);
        for first in (0..height).step_by(batch) {
            lines.clear();
            lines.extend(rows.by_ref().take(batch.min(height - first)));
            if lines.len() < batch.min(height - first) || lines.iter().any(|line| line.len() != width) {
                return Err(PoolIoError);
            }
            let at = self.offset + first * stride;
            if lines.len() > 1 {
                let slices: Vec<IoSlice<'_>> =
                    lines.iter().map(|line| IoSlice::new(bytemuck::cast_slice(line))).collect();
                if rustix::io::pwritev(&self.pool.fd, &slices, at as u64) == Ok(lines.len() * width * 4) {
                    continue;
                }
            }
            // One row at a time, also after a short `pwritev`: it tells a truncated pool from a write the kernel split.
            for (i, line) in lines.iter().enumerate() {
                self.pool
                    .fd
                    .write_all_at(bytemuck::cast_slice(line), (at + i * stride) as u64)
                    .map_err(|_| PoolIoError)?;
            }
        }
        Ok(())
    }
}

/// Returns the buffer's description when it is one of ours.
pub fn buffer_data(buffer: &WlBuffer) -> Option<&ShmBuffer> {
    buffer.data::<Arc<ShmBuffer>>().map(|b| b.as_ref())
}

fn accounting(client: &Client) -> Arc<Accounting> {
    client.get_data::<ClientState>().expect("every client is inserted with its state").accounting.clone()
}

pub fn tmpfs_backed(fd: &OwnedFd) -> bool {
    match rustix::fs::fstatfs(fd.as_fd()) {
        Ok(stat) => {
            let kind = stat.f_type as u64;
            kind == TMPFS_MAGIC || kind == HUGETLBFS_MAGIC
        }
        Err(_) => false,
    }
}

pub fn regular_file_size(fd: &impl AsFd) -> Option<u64> {
    let stat = rustix::fs::fstat(fd.as_fd()).ok()?;
    (rustix::fs::FileType::from_raw_mode(stat.st_mode) == rustix::fs::FileType::RegularFile)
        .then_some(stat.st_size as u64)
}

impl GlobalDispatch2<WlShm, State> for ShmGlobal {
    fn bind(
        &self,
        _state: &mut State,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<WlShm>,
        data_init: &mut DataInit<'_, State>,
    ) {
        let shm = data_init.init(resource, ShmGlobal);
        shm.format(wl_shm::Format::Argb8888);
        shm.format(wl_shm::Format::Xrgb8888);
    }
}

impl Dispatch2<WlShm, State> for ShmGlobal {
    fn request(
        &self,
        _state: &mut State,
        client: &Client,
        shm: &WlShm,
        request: wl_shm::Request,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        let wl_shm::Request::CreatePool { id, fd, size } = request else { return };
        let accounting = accounting(client);
        let Ok(size) = usize::try_from(size) else {
            shm.post_error(wl_shm::Error::InvalidStride, "invalid pool size");
            return;
        };
        if size == 0 || !tmpfs_backed(&fd) || regular_file_size(&fd).is_none_or(|len| len < size as u64) {
            shm.post_error(wl_shm::Error::InvalidFd, "pool must be a tmpfs file at least as large as declared");
            return;
        }
        let pools = accounting.pools.fetch_add(1, Ordering::Relaxed) + 1;
        let bytes = accounting.pool_bytes.fetch_add(size, Ordering::Relaxed) + size;
        let pool = data_init.init(id, Pool::new(fd, size, accounting));
        if pools > limits::MAX_POOLS || bytes > limits::MAX_POOL_BYTES {
            // The counters are released when the client's objects are destroyed with it.
            pool.post_error(wl_shm::Error::InvalidFd, "shared-memory limit exceeded");
        }
    }
}

impl Dispatch2<WlShmPool, State> for Arc<Pool> {
    fn request(
        &self,
        _state: &mut State,
        _client: &Client,
        pool: &WlShmPool,
        request: wl_shm_pool::Request,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        let accounting = &self.accounting;
        match request {
            wl_shm_pool::Request::CreateBuffer { id, offset, width, height, stride, format } => {
                let opaque = match format {
                    WEnum::Value(wl_shm::Format::Xrgb8888) => true,
                    WEnum::Value(wl_shm::Format::Argb8888) => false,
                    _ => {
                        pool.post_error(wl_shm::Error::InvalidFormat, "only ARGB8888 and XRGB8888");
                        return;
                    }
                };
                let geometry = buffer_geometry(offset, width, height, stride, *self.size.lock().unwrap());
                let Some((offset, width, height, stride)) = geometry else {
                    pool.post_error(wl_shm::Error::InvalidStride, "invalid buffer geometry");
                    return;
                };
                let (max_w, max_h) = crate::state::output_size();
                let buffers = accounting.buffers.fetch_add(1, Ordering::Relaxed) + 1;
                let buffer = data_init
                    .init(id, Arc::new(ShmBuffer { pool: self.clone(), offset, width, height, stride, opaque }));
                if width > max_w || height > max_h {
                    buffer.post_error(wl_shm::Error::InvalidStride, "buffer larger than the output");
                } else if buffers > limits::MAX_BUFFERS {
                    buffer.post_error(wl_shm::Error::InvalidStride, "buffer limit exceeded");
                }
            }
            wl_shm_pool::Request::Resize { size } => {
                let mut current = self.size.lock().unwrap();
                let Ok(size) = usize::try_from(size) else {
                    pool.post_error(wl_shm::Error::InvalidStride, "invalid pool size");
                    return;
                };
                if size < *current {
                    pool.post_error(wl_shm::Error::InvalidStride, "pools only grow");
                    return;
                }
                let grown = size - *current;
                let bytes = accounting.pool_bytes.fetch_add(grown, Ordering::Relaxed) + grown;
                *current = size;
                if bytes > limits::MAX_POOL_BYTES || regular_file_size(&self.fd).is_none_or(|len| len < size as u64) {
                    pool.post_error(wl_shm::Error::InvalidFd, "shared-memory limit exceeded or pool not grown");
                }
            }
            _ => {}
        }
    }

    fn destroyed(&self, _state: &mut State, _client: ClientId, _resource: &WlShmPool) {
        self.accounting.pools.fetch_sub(1, Ordering::Relaxed);
        self.accounting.pool_bytes.fetch_sub(*self.size.lock().unwrap(), Ordering::Relaxed);
    }
}

/// Validates a buffer inside a pool: positive size, a 4-byte-aligned stride of at least `width * 4` plus bounded
/// padding, and the whole rectangle inside the pool.
fn buffer_geometry(
    offset: i32,
    width: i32,
    height: i32,
    stride: i32,
    pool_size: usize,
) -> Option<(usize, u32, u32, u32)> {
    let (offset, width, height, stride) = (
        u32::try_from(offset).ok()?,
        u32::try_from(width).ok()?,
        u32::try_from(height).ok()?,
        u32::try_from(stride).ok()?,
    );
    if width == 0 || height == 0 || !stride.is_multiple_of(4) {
        return None;
    }
    let row = width.checked_mul(4)?;
    if stride < row || stride - row > limits::MAX_STRIDE_PADDING {
        return None;
    }
    let end = (offset as u64).checked_add((stride as u64) * (height as u64 - 1) + row as u64)?;
    (end <= pool_size as u64).then_some((offset as usize, width, height, stride))
}

impl Dispatch2<WlBuffer, State> for Arc<ShmBuffer> {
    fn request(
        &self,
        _state: &mut State,
        _client: &Client,
        _resource: &WlBuffer,
        _request: wl_buffer::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        // Destroy is the only request; `destroyed` does the accounting.
    }

    fn destroyed(&self, _state: &mut State, _client: ClientId, _resource: &WlBuffer) {
        self.pool.accounting.buffers.fetch_sub(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A buffer of `width` x `height` with `stride`, at `offset` of a memfd holding `bytes`.
    fn buffer(bytes: &[u8], offset: usize, width: u32, height: u32, stride: u32) -> ShmBuffer {
        let fd = rustix::fs::memfd_create("pool", rustix::fs::MemfdFlags::CLOEXEC).expect("memfd_create");
        rustix::io::pwrite(&fd, bytes, 0).expect("pwrite");
        let accounting = Arc::new(Accounting::default());
        let pool = Pool::new(fd, bytes.len(), accounting);
        ShmBuffer { pool, offset, width, height, stride, opaque: true }
    }

    #[test]
    fn rectangles_and_rows_move_whole_in_batches_and_a_truncated_pool_fails() {
        let (width, height, stride, offset) = (40u32, 150u32, 40 * 4 + 12, 8);
        let bytes: Vec<u8> = (0..offset + (stride * height) as usize).map(|i| (i * 7 + i / 251) as u8).collect();
        let pixel = |x: u32, y: u32| {
            let at = offset + (y * stride + x * 4) as usize;
            u32::from_ne_bytes(bytes[at..at + 4].try_into().expect("four bytes"))
        };
        let padded = buffer(&bytes, offset, width, height, stride);
        // More rows than one batch holds, columns off both edges, into a wider destination.
        let dst_width = 50;
        let mut dst = vec![0; dst_width * height as usize];
        padded.read_rect(3, 2, 30, 140, &mut dst, dst_width).expect("a readable pool");
        for y in 0..height {
            for x in 0..dst_width as u32 {
                let inside = (3..33).contains(&x) && (2..142).contains(&y);
                let expected = if inside { pixel(x, y) } else { 0 };
                assert_eq!(dst[(y as usize) * dst_width + x as usize], expected, "pixel {x},{y}");
            }
        }

        let frame: Vec<u32> = (0..width * height).map(|i| i.wrapping_mul(2654435761)).collect();
        let rows = || (0..height as usize).map(|y| &frame[y * width as usize..][..width as usize]);
        for stride in [width * 4, stride] {
            let target = buffer(&vec![0; offset + (stride * height) as usize], offset, width, height, stride);
            target.write_rows(rows()).expect("a writable pool");
            let mut back = vec![0; (width * height) as usize];
            target.read_rect(0, 0, width, height, &mut back, width as usize).expect("a readable pool");
            assert_eq!(back, frame, "stride {stride}");
            assert!(target.write_rows(rows().take(height as usize - 1)).is_err(), "too few rows");
        }

        rustix::fs::ftruncate(&padded.pool.fd, (offset + 100 * stride as usize) as u64).expect("truncate");
        assert!(padded.read_rect(3, 2, 30, 140, &mut dst, dst_width).is_err(), "a short read fails");
        assert!(padded.read_rect(3, 2, 30, 90, &mut dst, dst_width).is_ok(), "rows before the cut still read");
    }

    #[test]
    fn buffer_geometry_is_bounded() {
        assert_eq!(buffer_geometry(0, 4, 2, 16, 32), Some((0, 4, 2, 16)));
        // The last row needs only `width * 4` bytes.
        assert_eq!(buffer_geometry(0, 4, 2, 20, 36), Some((0, 4, 2, 20)));
        assert_eq!(buffer_geometry(0, 4, 2, 16, 31), None);
        assert_eq!(buffer_geometry(0, 4, 2, 15, 64), None);
        assert_eq!(buffer_geometry(0, 4, 2, 12, 64), None);
        assert_eq!(buffer_geometry(0, 4, 2, 16 + 260, 4096), None);
        assert_eq!(buffer_geometry(-1, 4, 2, 16, 64), None);
        assert_eq!(buffer_geometry(0, 0, 2, 16, 64), None);
        assert_eq!(buffer_geometry(i32::MAX, 4, 2, 16, usize::MAX), Some((i32::MAX as usize, 4, 2, 16)));
    }
}
