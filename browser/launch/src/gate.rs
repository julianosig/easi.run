//! Backpressure from the person's download to Chromium's network: the proxy forwarder asks the gate before it hands
//! Chromium any byte from the network, and the gate holds it back while downloads being received fall behind.
//!
//! It is liveness, not the safety bound (the kernel's tmpfs size is that): it keeps an ordinary large download at the
//! person's speed instead of failing with disk full. The gate is recomputed from fresh per-file facts, at least every
//! [`RECOMPUTE_BYTES`] forwarded and every [`RECOMPUTE_EVERY`]:
//! - a regular file in the downloads directory the files thread does not hold yet, past [`PRE`], closes it (however
//!   late its creation is handled), so a download is small when it is first streamed;
//! - the backlog of the downloads being received (allocated bytes, less their pinned prefix) past [`HARD`] closes it
//!   until it is below [`HIGH`]; past [`HIGH`] only as many bytes pass as the files thread handed to the display.

use std::collections::HashMap;
use std::fs::File;
use std::os::unix::fs::MetadataExt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const MIB: u64 = 1024 * 1024;
/// Allocation a file not yet held may reach before the gate closes.
pub const PRE: u64 = 4 * MIB;
/// Backlog past which only drained bytes pass, and past which nothing does.
const HIGH: u64 = 48 * MIB;
const HARD: u64 = 96 * MIB;
/// Credit a drained download may bank for the network.
const MAX_TOKENS: u64 = 4 * MIB;
const RECOMPUTE_BYTES: u64 = 256 * 1024;
const RECOMPUTE_EVERY: Duration = Duration::from_millis(10);
/// How long a held-back forwarder sleeps between looks.
const WAIT: Duration = Duration::from_millis(5);

/// A file the files thread holds open: its backlog counts only while someone receives it; one not yet admitted
/// (`pre`, Chromium's temporary file) stays under [`PRE`] as if it were not held.
pub struct Held {
    pub file: Arc<File>,
    pub pinned: u64,
    pub received: bool,
    pub pre: bool,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    Open,
    Credit,
    Closed,
}

struct Computed {
    mode: Mode,
    at: Instant,
    since: u64,
    tokens: u64,
    /// Closed by the backlog: stays closed until it is below [`HIGH`].
    hard: bool,
}

pub struct Gate {
    dir: String,
    /// By inode. The files thread removes an entry before it unlinks or drops the file.
    pub held: Mutex<HashMap<u64, Held>>,
    computed: Mutex<Computed>,
}

impl Gate {
    pub fn new(dir: &str) -> Arc<Gate> {
        Arc::new(Gate {
            dir: dir.to_owned(),
            held: Mutex::new(HashMap::new()),
            computed: Mutex::new(Computed { mode: Mode::Open, at: Instant::now(), since: 0, tokens: 0, hard: false }),
        })
    }

    /// The files thread handed `n` bytes of a received download to the display: as many may come from the network.
    pub fn drained(&self, n: u64) {
        let mut c = self.computed.lock().unwrap();
        c.tokens = (c.tokens + n).min(MAX_TOKENS);
    }

    /// Waits until `n` bytes may go from the network to Chromium.
    pub fn admit(&self, n: u64) {
        loop {
            {
                let mut c = self.computed.lock().unwrap();
                if c.since >= RECOMPUTE_BYTES || c.at.elapsed() >= RECOMPUTE_EVERY {
                    let (mode, hard) = self.compute(c.hard);
                    (c.mode, c.hard, c.at, c.since) = (mode, hard, Instant::now(), 0);
                }
                let admitted = match c.mode {
                    Mode::Open => true,
                    Mode::Credit if c.tokens >= n => {
                        c.tokens -= n;
                        true
                    }
                    _ => false,
                };
                if admitted {
                    c.since += n;
                    return;
                }
            }
            std::thread::sleep(WAIT);
        }
    }

    fn compute(&self, was_hard: bool) -> (Mode, bool) {
        let held = self.held.lock().unwrap();
        if self.unheld_past_pre(&held) {
            return (Mode::Closed, was_hard);
        }
        let backlog: u64 = held
            .values()
            .filter(|h| h.received)
            .map(|h| h.file.metadata().map(|m| m.blocks() * 512).unwrap_or(0).saturating_sub(h.pinned))
            .sum();
        let hard = backlog > HARD || (was_hard && backlog >= HIGH);
        let mode = if hard {
            Mode::Closed
        } else if backlog > HIGH {
            Mode::Credit
        } else {
            Mode::Open
        };
        (mode, hard)
    }

    /// A regular file in the directory, not held or not yet admitted, allocated past [`PRE`].
    fn unheld_past_pre(&self, held: &HashMap<u64, Held>) -> bool {
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return false };
        entries.flatten().any(|entry| {
            entry
                .metadata()
                .is_ok_and(|m| m.is_file() && held.get(&m.ino()).is_none_or(|h| h.pre) && m.blocks() * 512 > PRE)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn dir() -> String {
        let dir = std::env::temp_dir().join(format!("gate-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir(&dir).unwrap();
        dir.to_string_lossy().into_owned()
    }

    fn grow(path: &str, bytes: u64) -> File {
        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
        file.write_all(&vec![1u8; bytes as usize]).unwrap();
        file.sync_all().unwrap();
        file
    }

    fn mode(gate: &Gate) -> Mode {
        gate.compute(false).0
    }

    #[test]
    fn a_file_not_yet_held_closes_the_gate_past_its_allowance() {
        let dir = dir();
        let gate = Gate::new(&dir);
        let path = format!("{dir}/.org.chromium.Chromium.x");
        grow(&path, PRE / 2);
        assert_eq!(mode(&gate), Mode::Open);
        let file = grow(&path, PRE);
        assert_eq!(mode(&gate), Mode::Closed, "an undiscovered download past PRE holds the network back");
        let ino = file.metadata().unwrap().ino();
        gate.held.lock().unwrap().insert(ino, Held { file: Arc::new(file), pinned: 0, received: false, pre: true });
        assert_eq!(mode(&gate), Mode::Closed, "a temporary file not yet admitted still counts");
        gate.held.lock().unwrap().get_mut(&ino).unwrap().pre = false;
        assert_eq!(mode(&gate), Mode::Open, "once admitted, a download nobody receives is not held back");
    }

    #[test]
    fn a_received_backlog_passes_only_drained_bytes_then_nothing() {
        let dir = dir();
        let gate = Gate::new(&dir);
        let file = grow(&format!("{dir}/a.crdownload"), HIGH + MIB);
        let ino = file.metadata().unwrap().ino();
        let file = Arc::new(file);
        gate.held.lock().unwrap().insert(ino, Held { file: file.clone(), pinned: 0, received: true, pre: false });
        assert_eq!(mode(&gate), Mode::Credit);
        gate.held.lock().unwrap().get_mut(&ino).unwrap().pinned = 2 * MIB;
        assert_eq!(mode(&gate), Mode::Open, "a pinned prefix is not backlog");
        gate.held.lock().unwrap().get_mut(&ino).unwrap().pinned = 0;
        grow(&format!("{dir}/a.crdownload"), HARD);
        assert_eq!(gate.compute(false), (Mode::Closed, true));
        // Closed by the backlog, it reopens only below HIGH.
        rustix::fs::fallocate(
            &*file,
            rustix::fs::FallocateFlags::PUNCH_HOLE | rustix::fs::FallocateFlags::KEEP_SIZE,
            0,
            HARD + 2 * MIB,
        )
        .unwrap();
        assert_eq!(gate.compute(true), (Mode::Open, false));
    }
}
