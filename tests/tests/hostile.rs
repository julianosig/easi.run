//! Milestone B: the hostile-client requirements (docs/design.md §3.2), against the real `display` binary.
//!
//! Every test starts its own compositor on private sockets, lets a hostile client cross one bound, and asserts that
//! the compositor disconnects that client, stays up, and then serves a fresh well-behaved client end to end (a
//! toplevel with real shared-memory pixels and a frame callback).

use std::io::{Read, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use display_tests::compositor::{Compositor, Options, current_uid};
use display_tests::wayland::{
    Client, TIMEOUT, Window, assert_serves, commit_requests, connect_socket, memfd, raw_flood, request, send_with_fds,
    served, string_request, sync_request, write_at,
};
use wayland_client::Proxy;
use wayland_client::protocol::{wl_data_device_manager::WlDataDeviceManager, wl_output, wl_shm};
use wayland_protocols::xdg::shell::client::xdg_positioner::{self, XdgPositioner};

// The bounds of `display/src/limits.rs`; the tests prove each one from outside the process.
const MAX_POOLS: usize = 32;
const MAX_POOL_BYTES: usize = 64 * 1024 * 1024;
const MAX_SURFACES: usize = 64;
const MAX_SUBSURFACES: usize = 32;
const MAX_PENDING_SUBSURFACE_COMMITS: usize = 32;
const MAX_STRIDE_PADDING: i32 = 256;
const MAX_UNUSED_FDS: usize = 1024;
const MAX_OBJECTS: usize = 4096;
const MAX_CLIENT_ID: u32 = 16384;
const MAX_BROWSER_CLIENTS: usize = 16;
const MAX_POSITIONER_EXTENT: i32 = 1 << 20;
const MAX_PIXEL_OUTPUTS: i32 = 8;
// The output every test runs with unless it says otherwise.
const OUTPUT: (i32, i32) = (320, 240);

/// Runs `attack` on a fresh client and asserts that the compositor disconnects it and survives.
fn hostile(
    compositor: &mut Compositor,
    socket_of: fn(&Compositor) -> &std::path::Path,
    attack: impl FnOnce(&mut Client),
) {
    let mut client = Client::connect(socket_of(compositor));
    attack(&mut client);
    // A protocol error when the compositor names the offending object, EOF when it drops the connection.
    let why = client.expect_disconnect(TIMEOUT);
    eprintln!("disconnected: {why}");
    compositor.assert_alive();
}

fn browser(compositor: &Compositor) -> &std::path::Path {
    &compositor.browser_socket
}

fn agent(compositor: &Compositor) -> &std::path::Path {
    compositor.agent_socket.as_deref().expect("an agent socket")
}

/// The ending of every test: the compositor is up and serves the next client.
fn serves_next(compositor: &mut Compositor) {
    compositor.assert_alive();
    assert_serves(&compositor.browser_socket);
    compositor.assert_alive();
}

#[test]
fn pools_over_the_size_or_count_limit_disconnect() {
    let mut compositor = Compositor::start();
    // A pool of exactly the byte limit is accepted (and stays alive beside the hostile clients: the limit is per
    // client). The memfds are sparse, so no test allocates this memory.
    let mut neighbour = Client::connect(&compositor.browser_socket);
    let _at_limit = neighbour.pool(MAX_POOL_BYTES);
    neighbour.sync().expect("a pool of exactly the byte limit is accepted");

    hostile(&mut compositor, browser, |c| drop(c.pool(MAX_POOL_BYTES + 4096)));
    hostile(&mut compositor, browser, |c| {
        let first = c.pool(MAX_POOL_BYTES / 2 + 4096);
        c.sync().expect("the first half is accepted");
        let second = c.pool(MAX_POOL_BYTES / 2);
        drop((first, second));
    });
    hostile(&mut compositor, browser, |c| {
        let fd = memfd(4096);
        for _ in 0..MAX_POOLS {
            c.shm.create_pool(fd.as_fd(), 4096, &c.qh, ());
        }
        c.sync().expect("MAX_POOLS pools are accepted");
        c.shm.create_pool(fd.as_fd(), 4096, &c.qh, ());
    });
    for size in [0, -4096] {
        hostile(&mut compositor, browser, |c| {
            let fd = memfd(4096);
            c.shm.create_pool(fd.as_fd(), size, &c.qh, ());
        });
    }

    neighbour.sync().expect("the neighbour is unaffected");
    serves_next(&mut compositor);
}

#[test]
fn pool_descriptors_that_are_not_tmpfs_files_disconnect() {
    let mut compositor = Compositor::start();
    let mut offer = |fd: OwnedFd, size: i32| {
        hostile(&mut compositor, browser, |c| {
            c.shm.create_pool(fd.as_fd(), size, &c.qh, ());
        })
    };

    // A regular file on a disk file system: a read could block on it, so it is refused.
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    const TMPFS_MAGIC: u64 = 0x0102_1994;
    let on_disk = rustix::fs::statfs(dir).map(|s| s.f_type as u64 != TMPFS_MAGIC).unwrap_or(false);
    if on_disk {
        let path = dir.join(format!("hostile-pool-{}", std::process::id()));
        let file = std::fs::File::create(&path).expect("a regular file");
        file.set_len(4096).expect("size it");
        std::fs::remove_file(&path).expect("unlink it");
        offer(file.into(), 4096);
    } else {
        eprintln!("skipped the disk-file case: {} is tmpfs", dir.display());
    }
    let (read, _write) = rustix::pipe::pipe().expect("a pipe");
    offer(read, 4096);
    let (socket, _peer) = UnixStream::pair().expect("a socket pair");
    offer(socket.into(), 4096);
    // `/dev` is devtmpfs, which reports the tmpfs magic: only the regular-file check stops a device.
    offer(std::fs::File::open("/dev/zero").expect("/dev/zero").into(), 4096);
    // A memfd shorter than the pool it claims to be.
    offer(memfd(4096), 8192);

    serves_next(&mut compositor);
}

#[test]
fn buffers_with_bad_geometry_disconnect() {
    let mut compositor = Compositor::start();
    const POOL: i32 = 64 * 1024;
    // At the bounds: padding of exactly MAX_STRIDE_PADDING, a buffer ending exactly at the pool's end.
    let mut fine = Client::connect(&compositor.browser_socket);
    let pool = fine.pool(POOL as usize);
    fine.buffer(&pool, 0, 16, 16, 64 + MAX_STRIDE_PADDING);
    fine.buffer(&pool, 0, 128, 128, 512);
    fine.buffer(&pool, POOL - 64, 16, 1, 64);
    fine.sync().expect("buffers at the bounds are accepted");

    let bad: &[(&str, i32, i32, i32, i32)] = &[
        ("stride below width * 4", 0, 16, 16, 60),
        ("stride not a multiple of 4", 0, 16, 16, 66),
        ("padding beyond the bound", 0, 16, 16, 64 + MAX_STRIDE_PADDING + 4),
        ("zero width", 0, 0, 16, 64),
        ("zero height", 0, 16, 0, 64),
        ("negative width", 0, -16, 16, 64),
        ("negative height", 0, 16, -16, 64),
        ("negative stride", 0, 16, 16, -64),
        ("negative offset", -64, 16, 16, 64),
        ("last row past the pool's end", 0, 128, 129, 512),
        ("offset past the pool's end", POOL - 60, 16, 1, 64),
        ("offset at i32::MAX", i32::MAX, 16, 1, 64),
    ];
    for &(case, offset, width, height, stride) in bad {
        eprintln!("case: {case}");
        hostile(&mut compositor, browser, |c| {
            let pool = c.pool(POOL as usize);
            c.buffer(&pool, offset, width, height, stride);
        });
    }
    hostile(&mut compositor, browser, |c| {
        let pool = c.pool(POOL as usize);
        pool.pool.create_buffer(0, 16, 16, 32, wl_shm::Format::Rgb565, &c.qh, ());
    });

    fine.sync().expect("the well-formed client is unaffected");
    serves_next(&mut compositor);
}

#[test]
fn buffers_larger_than_the_output_disconnect() {
    let mut compositor = Compositor::start();
    let (w, h) = OUTPUT;
    let mut fine = Client::connect(&compositor.browser_socket);
    let pool = fine.pool((w * h * 4) as usize);
    fine.buffer(&pool, 0, w, h, w * 4);
    fine.sync().expect("a buffer of exactly the output's size is accepted");

    for (width, height) in [(w + 1, 1), (1, h + 1), (w + 1, h + 1)] {
        hostile(&mut compositor, browser, |c| {
            let pool = c.pool((width * height * 4) as usize);
            c.buffer(&pool, 0, width, height, width * 4);
        });
    }
    serves_next(&mut compositor);
}

/// The `SIGBUS` case: a pool shrunk under the compositor between a commit and the frame that reads it.
#[test]
fn a_pool_truncated_before_the_frame_disconnects() {
    // One composition and one frame per second leave a wide, deterministic window between the commit and the read.
    let mut compositor = Compositor::start_with(Options { max_fps: 1, compose_gap_ms: 1000, ..Options::default() });
    for truncate_to in [0, 64 * 48 * 4 / 2] {
        hostile(&mut compositor, browser, |c| {
            let window = c.window().expect("a toplevel");
            let (w, h) = (64, 48);
            let pool = c.pool(w * h * 4);
            write_at(&pool.fd, &vec![0x40; w * h * 4], 0);
            let buffer = c.buffer(&pool, 0, w as i32, h as i32, w as i32 * 4);
            let first = c.present(&window.surface, &buffer, w as i32, h as i32);
            c.wait(Duration::from_secs(3), |app| app.done.contains(&first)).expect("the first frame");
            // The next frame is now about a second away: commit, then pull the memory out from under it.
            c.present(&window.surface, &buffer, w as i32, h as i32);
            c.flush().expect("the commit is sent");
            c.sync().expect("the commit is processed");
            rustix::fs::ftruncate(&pool.fd, truncate_to as u64).expect("truncate the pool");
        });
    }
    // Both disconnects came from the short read, not from another check.
    let log = compositor.log();
    assert_eq!(log.matches("message: \"unreadable shared memory\"").count(), 2, "{log}");
    serves_next(&mut compositor);
}

#[test]
fn pool_resizes_that_shrink_or_cross_the_limit_disconnect() {
    let mut compositor = Compositor::start();
    let mut fine = Client::connect(&compositor.browser_socket);
    let pool = fine.pool(4096);
    rustix::fs::ftruncate(&pool.fd, 8192).expect("grow the file");
    pool.pool.resize(8192);
    fine.buffer(&pool, 4096, 16, 16, 64);
    fine.sync().expect("a pool grown with its file is accepted, and its new bytes are usable");

    hostile(&mut compositor, browser, |c| {
        let pool = c.pool(8192);
        pool.pool.resize(4096);
    });
    hostile(&mut compositor, browser, |c| {
        let pool = c.pool(4096);
        rustix::fs::ftruncate(&pool.fd, (MAX_POOL_BYTES + 8192) as u64).expect("grow the file");
        pool.pool.resize((MAX_POOL_BYTES + 8192) as i32);
    });
    hostile(&mut compositor, browser, |c| {
        let pool = c.pool(4096);
        pool.pool.resize(8192);
    });
    hostile(&mut compositor, browser, |c| {
        let pool = c.pool(4096);
        pool.pool.resize(-1);
    });
    fine.sync().expect("the well-formed client is unaffected");
    serves_next(&mut compositor);
}

#[test]
fn surface_and_subsurface_floods_disconnect() {
    let mut compositor = Compositor::start();
    hostile(&mut compositor, browser, |c| {
        let surfaces: Vec<_> = (0..MAX_SURFACES).map(|_| c.surface()).collect();
        c.sync().expect("MAX_SURFACES surfaces are accepted");
        for _ in 0..MAX_SURFACES {
            c.surface();
        }
        drop(surfaces);
    });
    hostile(&mut compositor, browser, |c| {
        let parent = c.surface();
        let mut children = Vec::new();
        for _ in 0..MAX_SUBSURFACES {
            let child = c.surface();
            children.push(c.subcompositor.get_subsurface(&child, &parent, &c.qh, ()));
        }
        c.sync().expect("MAX_SUBSURFACES subsurfaces are accepted");
        for _ in 0..MAX_SUBSURFACES {
            let child = c.surface();
            children.push(c.subcompositor.get_subsurface(&child, &parent, &c.qh, ()));
        }
    });
    serves_next(&mut compositor);
}

/// A browser gives the same surface a subsurface again each time it shows it and destroys that subsurface each time it
/// hides it: a surface counts once against the subsurface bound, however often it is made a subsurface.
#[test]
fn a_surface_made_a_subsurface_again_and_again_counts_once() {
    let mut compositor = Compositor::start();
    let mut client = Client::connect(&compositor.browser_socket);
    let parent = client.surface();
    let child = client.surface();
    for _ in 0..MAX_SUBSURFACES * 4 {
        client.subcompositor.get_subsurface(&child, &parent, &client.qh, ()).destroy();
        client.sync().expect("a shown and hidden subsurface is no flood");
    }
    drop(client);
    serves_next(&mut compositor);
}

/// Objects a client creates and never commits or destroys (here frame callbacks; presentation feedback and regions
/// count the same) are bounded by one live-object limit across every interface.
#[test]
fn an_object_flood_disconnects() {
    let mut compositor = Compositor::start();
    hostile(&mut compositor, browser, |c| {
        let surface = c.surface();
        for _ in 0..MAX_OBJECTS / 2 {
            let token = c.token();
            surface.frame(&c.qh, token);
        }
        c.sync().expect("half the object limit is accepted");
        for _ in 0..MAX_OBJECTS {
            let token = c.token();
            surface.frame(&c.qh, token);
        }
    });
    serves_next(&mut compositor);
}

/// Objects created and destroyed under ever higher ids keep the live count low but would grow the library's id map:
/// an id above the limit disconnects. The attack is written raw, since a well-behaved library reuses free ids.
#[test]
fn ever_higher_object_ids_disconnect() {
    let mut compositor = Compositor::start();
    let stream = connect_socket(&compositor.browser_socket);
    let mut flood = Vec::new();
    // wl_display.sync (opcode 0) creates a callback the server destroys at once; ids 2.. never repeat.
    for id in 2..MAX_CLIENT_ID + 64 {
        flood.extend(sync_request(id));
    }
    let _ = (&stream).write_all(&flood);
    stream.set_read_timeout(Some(TIMEOUT)).expect("a read timeout");
    let mut events = Vec::new();
    let ended = (&stream).take(64 << 20).read_to_end(&mut events);
    assert!(ended.is_ok(), "the compositor disconnects the client: {ended:?}");
    compositor.assert_alive();
    serves_next(&mut compositor);
}

/// A client may allocate ids only in its own namespace: a request that creates an object under a server-namespace id
/// disconnects it (third_party/README.md: the library's patch).
#[test]
fn server_namespace_ids_disconnect() {
    let mut compositor = Compositor::start();
    let stream = connect_socket(&compositor.browser_socket);
    let flood: Vec<u8> = (0xff00_0000u32..0xff00_0040).flat_map(sync_request).collect();
    let _ = (&stream).write_all(&flood);
    stream.set_read_timeout(Some(TIMEOUT)).expect("a read timeout");
    let mut events = Vec::new();
    let ended = (&stream).take(1 << 20).read_to_end(&mut events);
    assert!(ended.is_ok(), "the compositor disconnects the client: {ended:?}");
    compositor.assert_alive();
    serves_next(&mut compositor);
}

/// A well-behaved client that measures how fast the compositor answers.
struct Probe {
    client: Client,
    window: Window,
}

impl Probe {
    fn new(compositor: &Compositor) -> Probe {
        let mut client = Client::connect(&compositor.browser_socket);
        let window = client.window().expect("the probe's toplevel");
        let pool = client.pool(64 * 64 * 4);
        let buffer = client.buffer(&pool, 0, 64, 64, 256);
        let first = client.present(&window.surface, &buffer, 64, 64);
        client.wait(TIMEOUT, |app| app.done.contains(&first)).expect("the probe's first frame");
        Probe { client, window }
    }

    /// Asks for one frame after another while `running` holds; each must arrive within a second. Returns how many
    /// arrived and the slowest.
    fn frames_while(&mut self, running: &AtomicBool) -> (usize, Duration) {
        self.answers_while(running, "frame callback", |probe, token| {
            probe.window.surface.frame(&probe.client.qh, token);
            probe.window.surface.commit();
        })
    }

    /// Makes one `wl_display.sync` round trip after another while `running` holds, for a probe whose window is
    /// hidden and so gets no frames; each must be answered within a second. Returns how many and the slowest.
    fn syncs_while(&mut self, running: &AtomicBool) -> (usize, Duration) {
        self.answers_while(running, "sync answer", |probe, token| {
            probe.client.conn.display().sync(&probe.client.qh, token);
        })
    }

    /// Sends what `ask` writes for a fresh callback token while `running` holds and waits at most a second for that
    /// callback each time. Returns how many came back and the slowest.
    fn answers_while(&mut self, running: &AtomicBool, what: &str, ask: impl Fn(&Probe, u32)) -> (usize, Duration) {
        let (mut answers, mut slowest) = (0, Duration::ZERO);
        while running.load(Ordering::SeqCst) {
            let token = self.client.token();
            ask(self, token);
            let sent = Instant::now();
            self.client
                .wait(Duration::from_secs(1), |app| app.done.contains(&token))
                .unwrap_or_else(|why| panic!("no {what} within 1 s during the flood: {why}"));
            slowest = slowest.max(sent.elapsed());
            answers += 1;
        }
        (answers, slowest)
    }
}

const FLOOD: Duration = Duration::from_secs(2);

/// A tight loop of full-damage commits on the shown window, written straight onto the socket, so every commit is
/// composed, copied, and encoded: another client keeps getting answers within a second, and the compositor's memory
/// stays bounded however many commits arrive.
#[test]
fn a_commit_flood_keeps_the_compositor_responsive_and_bounded() {
    let mut compositor = Compositor::start();
    let (w, h) = OUTPUT;
    // The probe's window opens first, so the flood's window opens over it and is the one shown and composed. The
    // hidden probe gets no frames, so it measures the compositor with sync round trips.
    let mut probe = Probe::new(&compositor);

    let mut flood = Client::connect(&compositor.browser_socket);
    let window = flood.window().expect("the flood's toplevel");
    let pool = flood.pool((w * h * 4) as usize);
    write_at(&pool.fd, &vec![0x7f; (w * h * 4) as usize], 0);
    let buffer = flood.buffer(&pool, 0, w, h, w * 4);
    let first = flood.present(&window.surface, &buffer, w, h);
    flood.wait(TIMEOUT, |app| app.done.contains(&first)).expect("the flood's first frame");
    let commit = commit_requests(&window.surface, &buffer, (w, h));
    let commit_bytes = commit.len();
    let rss_before = compositor.rss_kib();

    let flooding = Arc::new(AtomicBool::new(true));
    let attacker = {
        let flooding = flooding.clone();
        std::thread::spawn(move || {
            let fd = flood.conn.as_fd();
            let result = raw_flood(fd, FLOOD, |chunk| chunk.extend(commit.repeat(1024)));
            flooding.store(false, Ordering::SeqCst);
            result
        })
    };
    let (syncs, slowest) = probe.syncs_while(&flooding);
    let (sent, error) = attacker.join().expect("the flood thread");
    let rss_after = compositor.rss_kib();
    eprintln!(
        "commit flood: {} commits in {FLOOD:?} (ended by {error:?}); {syncs} probe syncs, slowest {slowest:?}; \
         RSS {rss_before} KiB -> {rss_after} KiB",
        sent / commit_bytes
    );
    assert!(sent / commit_bytes >= 1000, "thousands of commits were sent ({})", sent / commit_bytes);
    assert!(syncs >= 10, "the probe ran during the flood ({syncs} syncs)");
    assert!(
        rss_after.saturating_sub(rss_before) < 64 * 1024,
        "RSS grew from {rss_before} KiB to {rss_after} KiB under the flood"
    );
    serves_next(&mut compositor);
}

/// Floods one request straight onto a client's socket for [`FLOOD`], each case in its own compositor so no case reuses
/// memory another freed; returns the requests sent, whether the compositor closed the connection, and how much its
/// memory grew.
fn list_flood(one: impl FnOnce(&mut Client) -> Vec<u8>) -> (usize, bool, u64) {
    let mut compositor = Compositor::start();
    let mut client = Client::connect(&compositor.browser_socket);
    let one = one(&mut client);
    client.sync().expect("the objects are created");
    let rss_before = compositor.rss_kib();
    let (sent, error) = raw_flood(client.conn.as_fd(), FLOOD, |chunk| chunk.extend(one.repeat(1024)));
    let grown = compositor.rss_kib().saturating_sub(rss_before);
    serves_next(&mut compositor);
    (sent / one.len(), error.is_some(), grown)
}

/// Requests that only add to a list the compositor keeps until a commit, or for the object's life: a flood of
/// `wl_surface.damage` without a commit collapses into whole-surface damage, and one of `wl_region.add` or
/// `wl_data_source.offer` disconnects the client. The compositor's memory stays bounded either way.
#[test]
fn request_lists_without_a_commit_stay_bounded() {
    let damage = list_flood(|c| request(c.surface().id().protocol_id(), 2, &[0, 0, 1, 1]));
    let region = list_flood(|c| {
        let region = c.compositor.create_region(&c.qh, ());
        request(region.id().protocol_id(), 1, &[0, 0, 1, 1])
    });
    let offer = list_flood(|c| {
        let manager = c.globals.bind::<WlDataDeviceManager, _, _>(&c.qh, 1..=3, ()).expect("wl_data_device_manager");
        let source = manager.create_data_source(&c.qh, ());
        string_request(source.id().protocol_id(), 0, "text/x-hostile")
    });
    let cases = [("damage", damage), ("region", region), ("offer", offer)];
    for (case, (sent, closed, grown)) in cases {
        eprintln!("{case} flood: {sent} requests, closed {closed}, RSS +{grown} KiB");
    }
    for (case, (_, _, grown)) in cases {
        assert!(grown < 4 * 1024, "the {case} flood grew the compositor by {grown} KiB");
    }
    assert!(damage.0 >= 100_000 && !damage.1, "damage is valid however much of it comes: {damage:?}");
    assert!(region.1 && offer.1, "region and offer floods disconnect: {region:?} {offer:?}");
}

/// A synchronized subsurface's commit waits for its parent's, and the compositor keeps each one until then: a flood
/// of damage and commits on one whose parent never commits disconnects the client past a few pending commits, and the
/// compositor's memory stays bounded. Commits the parent's commit applies no longer count.
#[test]
fn synchronized_subsurface_commits_pending_their_parent_stay_bounded() {
    let mut compositor = Compositor::start();
    let mut client = Client::connect(&compositor.browser_socket);
    let parent = client.surface();
    let child = client.surface();
    let _subsurface = client.subcompositor.get_subsurface(&child, &parent, &client.qh, ());
    for _ in 0..4 * MAX_PENDING_SUBSURFACE_COMMITS {
        for _ in 0..MAX_PENDING_SUBSURFACE_COMMITS {
            child.damage_buffer(0, 0, 1, 1);
            child.commit();
        }
        parent.commit();
    }
    client.sync().expect("commits the parent's commit applies are accepted however many come");
    drop(client);

    // A parent's commit that applies an older commit of the child, and none of the newer ones, leaves them counted.
    hostile(&mut compositor, browser, |c| {
        let (root, parent, child) = (c.surface(), c.surface(), c.surface());
        let middle = c.subcompositor.get_subsurface(&parent, &root, &c.qh, ());
        let leaf = c.subcompositor.get_subsurface(&child, &parent, &c.qh, ());
        child.commit();
        parent.commit(); // the parent's pending transaction now holds the child's first commit
        middle.set_desync();
        leaf.set_desync();
        child.commit(); // applied at once, with the first
        leaf.set_sync();
        for _ in 0..MAX_PENDING_SUBSURFACE_COMMITS {
            child.damage_buffer(0, 0, 1, 1);
            child.commit();
        }
        leaf.set_desync();
        parent.commit(); // applies the held transaction, which has none of the child's newer commits
        leaf.set_sync();
        child.damage_buffer(0, 0, 1, 1);
        child.commit();
    });

    let (sent, closed, grown) = list_flood(|c| {
        let parent = c.surface();
        let child = c.surface();
        c.subcompositor.get_subsurface(&child, &parent, &c.qh, ());
        let id = child.id().protocol_id();
        [request(id, 9, &[0, 0, 1, 1]), request(id, 6, &[])].concat()
    });
    eprintln!("synchronized subsurface flood: {sent} damage and commit pairs, closed {closed}, RSS +{grown} KiB");
    assert!(closed, "the flood disconnects");
    assert!(grown < 4 * 1024, "the flood grew the compositor by {grown} KiB");
    serves_next(&mut compositor);
}

/// The same for a request the protocol library answers by itself (`wl_display.sync`), from a client that reads
/// every reply: no client holds the event loop.
#[test]
fn a_request_flood_keeps_the_compositor_responsive() {
    let mut compositor = Compositor::start();
    let mut probe = Probe::new(&compositor);
    let stream = connect_socket(&compositor.browser_socket);
    let flooding = Arc::new(AtomicBool::new(true));
    let attacker = {
        let flooding = flooding.clone();
        std::thread::spawn(move || {
            let mut id = 1u32;
            let result = raw_flood(stream.as_fd(), FLOOD, |chunk| {
                for _ in 0..1024 {
                    id += 1;
                    chunk.extend(sync_request(id));
                }
            });
            flooding.store(false, Ordering::SeqCst);
            result
        })
    };
    let (frames, slowest) = probe.frames_while(&flooding);
    let (sent, error) = attacker.join().expect("the flood thread");
    eprintln!("sync flood: {} syncs (ended by {error:?}); {frames} probe frames, slowest {slowest:?}", sent / 12);
    assert!(frames >= 10, "the probe ran during the flood ({frames} frames)");
    serves_next(&mut compositor);
}

/// Thousands of pools on fresh memfds, created and destroyed: the compositor keeps no descriptor of a destroyed
/// pool, including one a buffer outlived, and frees every descriptor of a client it disconnects.
#[test]
fn a_pool_descriptor_storm_leaves_no_descriptor_behind() {
    let mut compositor = Compositor::start();
    let mut storm = Client::connect(&compositor.browser_socket);
    storm.sync().expect("connected");
    let baseline = compositor.fd_count();
    for round in 0..5_000 {
        let pool = storm.pool(4096);
        if round % 2 == 0 {
            let buffer = storm.buffer(&pool, 0, 16, 16, 64);
            pool.pool.destroy();
            buffer.destroy();
        } else {
            pool.pool.destroy();
        }
        drop(pool.fd);
        if round % 100 == 99 {
            storm.sync().expect("the storm client stays connected: destroyed pools are released");
        }
    }
    storm.sync().expect("the storm client stays connected");
    let after_storm = compositor.fd_count();
    assert!(after_storm <= baseline, "descriptors leaked: {baseline} before, {after_storm} after the storm");

    // The same storm without destroying: the client crosses MAX_POOLS and is disconnected with its pools.
    hostile(&mut compositor, browser, |c| {
        for _ in 0..MAX_POOLS + 8 {
            let pool = c.pool(4096);
            std::mem::forget(pool.pool);
        }
    });
    wait_for_fds(&compositor, baseline);
    serves_next(&mut compositor);
}

/// Waits until the compositor holds no more than `limit` descriptors.
fn wait_for_fds(compositor: &Compositor, limit: usize) {
    let deadline = Instant::now() + TIMEOUT;
    while compositor.fd_count() > limit {
        assert!(Instant::now() < deadline, "descriptors stayed open: {} > {limit}", compositor.fd_count());
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Descriptors attached to requests that take none: the compositor holds at most `MAX_UNUSED_FDS` of them for a
/// client, then disconnects it and frees them all.
#[test]
fn stray_descriptors_are_bounded_and_disconnect() {
    let mut compositor = Compositor::start();
    let baseline = compositor.fd_count();
    let stream = connect_socket(&compositor.browser_socket);
    stream.set_nonblocking(true).expect("non-blocking");
    let null = std::fs::File::open("/dev/null").expect("/dev/null");
    let fds = [null.as_fd(); 28];
    let (mut peak, mut sent, mut closed) = (0, 0, None);
    let mut sink = [0u8; 4096];
    for id in 2u32..2_002 {
        match send_with_fds(&stream, &sync_request(id), &fds) {
            Ok(_) => sent += 1,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(1)),
            Err(e) => {
                closed = Some(e);
                break;
            }
        }
        while let Ok(n) = (&stream).read(&mut sink) {
            if n == 0 {
                break;
            }
        }
        if id % 8 == 0 {
            peak = peak.max(compositor.fd_count());
        }
    }
    compositor.assert_alive();
    eprintln!("stray descriptors: {sent} messages of 28 sent, ended by {closed:?}; fds {baseline} -> peak {peak}");
    assert!(closed.is_some(), "the client sending stray descriptors stayed connected");
    // The relay's own sockets, the bound, and one message in flight.
    assert!(
        peak <= baseline + 8 + MAX_UNUSED_FDS + 28,
        "the compositor hoarded stray descriptors: {baseline} -> {peak}"
    );
    wait_for_fds(&compositor, baseline);
    serves_next(&mut compositor);
}

#[test]
fn scaled_or_transformed_buffers_disconnect() {
    let mut compositor = Compositor::start();
    type Setup = fn(&wayland_client::protocol::wl_surface::WlSurface);
    let setups: [Setup; 3] = [
        |s| s.set_buffer_scale(2),
        |s| s.set_buffer_transform(wl_output::Transform::_90),
        |s| s.set_buffer_transform(wl_output::Transform::Flipped),
    ];
    for setup in setups {
        hostile(&mut compositor, browser, |c| {
            let window = c.window().expect("a toplevel");
            let pool = c.pool(64 * 64 * 4);
            let buffer = c.buffer(&pool, 0, 64, 64, 256);
            setup(&window.surface);
            c.present(&window.surface, &buffer, 64, 64);
        });
    }
    serves_next(&mut compositor);
}

/// The compositor checks the peer uid of every connection. The real cross-uid topology (the browser container's
/// uid A against the display's uid B) is proven by the container tests in security.rs; here the browser socket is
/// configured for another uid than the test's, so the same check refuses this process.
#[test]
fn a_connection_from_another_uid_is_refused() {
    let me = current_uid();
    let mut compositor =
        Compositor::start_with(Options { browser_uid: me.wrapping_add(1), agent_uid: Some(me), ..Options::default() });
    let mut stream = connect_socket(&compositor.browser_socket);
    stream.set_read_timeout(Some(TIMEOUT)).expect("a read timeout");
    let mut byte = [0u8; 1];
    match stream.read(&mut byte) {
        Ok(0) => {}
        Ok(_) => panic!("the compositor spoke to a connection from another uid"),
        Err(e) => assert!(
            e.kind() == std::io::ErrorKind::ConnectionReset,
            "the connection from another uid was not closed: {e}"
        ),
    }
    // The refused connection never got a client: a Wayland handshake on it fails.
    assert!(!served(&compositor.browser_socket), "a Wayland client from another uid was served");
    compositor.assert_alive();
    // The compositor still serves the uid it accepts.
    Client::connect(agent(&compositor)).sync().expect("the accepted uid is served");
    compositor.assert_alive();
}

#[test]
fn a_toplevel_from_the_agent_socket_disconnects() {
    let mut compositor = Compositor::start_with(Options { agent_uid: Some(current_uid()), ..Options::default() });
    hostile(&mut compositor, agent, |c| {
        let surface = c.surface();
        let xdg_surface = c.wm_base.get_xdg_surface(&surface, &c.qh, ());
        let toplevel = xdg_surface.get_toplevel(&c.qh, ());
        surface.commit();
        drop((xdg_surface, toplevel));
    });
    // The agent may still connect to observe, and the browser's toplevel still works.
    Client::connect(agent(&compositor)).sync().expect("the agent connects again");
    serves_next(&mut compositor);
}

#[test]
fn a_connection_flood_is_bounded_per_socket() {
    let mut compositor = Compositor::start();
    let mut clients: Vec<Client> =
        (0..MAX_BROWSER_CLIENTS).map(|_| Client::connect(&compositor.browser_socket)).collect();
    for client in &mut clients {
        client.sync().expect("a client within the limit is served");
    }
    // One more connection on the same socket is refused, and the compositor stays up.
    assert!(!served(&compositor.browser_socket), "a client over the per-socket limit was served");
    compositor.assert_alive();
    // Closing one frees its slot for the next client, once the compositor has seen the hang-up (later under load).
    drop(clients.pop());
    let deadline = Instant::now() + TIMEOUT;
    while !served(&compositor.browser_socket) {
        assert!(Instant::now() < deadline, "a freed slot never served the next client");
        std::thread::sleep(Duration::from_millis(20));
    }
    compositor.assert_alive();
}

/// The agent's virtual keyboard checks its keymap's descriptor the way pools are checked, and never maps, reads, or
/// compiles the keymap: a declared size past the file's end (which a mapping turns into a `SIGBUS`), past the bound, or
/// on a file that is not tmpfs disconnects the agent, as does a key before a keymap or beyond the evdev range. A real
/// keymap is accepted, and its keys reach the browser's focused window.
#[test]
fn virtual_keyboard_keymaps_are_checked_within_their_bounds() {
    const XKB_V1: u32 = 1;
    let mut compositor = Compositor::start_with(Options { agent_uid: Some(current_uid()), ..Options::default() });
    let mut browser = Client::connect(&compositor.browser_socket);
    let _keyboard = browser.keyboard();
    let window = browser.window().expect("a toplevel");
    let pool = browser.pool(64 * 64 * 4);
    let buffer = browser.buffer(&pool, 0, 64, 64, 256);
    let shown = browser.present(&window.surface, &buffer, 64, 64);
    browser.wait(TIMEOUT, |app| app.done.contains(&shown) && app.keymap.is_some()).expect("shown, with a keymap");
    let (keymap, size) = browser.app.keymap.take().expect("the seat's keymap");
    let mut text = vec![0; size as usize];
    rustix::io::pread(&keymap, &mut text, 0).expect("read the seat's keymap");

    // The SIGBUS case: a keymap file shorter than its declared size; then a declared size past the bound.
    hostile(&mut compositor, agent, |c| {
        let fd = memfd(4096);
        write_at(&fd, &text[..4096], 0);
        c.virtual_keyboard().keymap(XKB_V1, fd.as_fd(), 128 * 1024);
    });
    hostile(&mut compositor, agent, |c| {
        let fd = memfd(1 << 20);
        write_at(&fd, &text, 0);
        c.virtual_keyboard().keymap(XKB_V1, fd.as_fd(), 1 << 20);
    });
    hostile(&mut compositor, agent, |c| {
        let (read, write) = rustix::pipe::pipe().expect("a pipe");
        let _ = rustix::io::write(&write, &text[..1024]);
        c.virtual_keyboard().keymap(XKB_V1, read.as_fd(), 1024);
    });
    hostile(&mut compositor, agent, |c| c.virtual_keyboard().key(0, 30, 1));
    hostile(&mut compositor, agent, |c| {
        let keyboard = c.virtual_keyboard();
        keyboard.keymap(XKB_V1, keymap.as_fd(), size);
        keyboard.key(0, 0x300, 1);
    });

    // A few hundred bytes whose compilation takes gigabytes (a key code near 2^28) or aborts libxkbcommon (a shift
    // level near 2^31): the compositor never compiles the agent's keymap, so neither costs it anything.
    let rss_before = compositor.rss_kib();
    for bomb in [
        "xkb_keymap { xkb_keycodes { <A> = 268435455; }; xkb_types { include \"complete\" }; \
         xkb_compat { include \"complete\" }; xkb_symbols { key <A> { [a] }; }; };",
        "xkb_keymap { xkb_keycodes { <A> = 9; }; xkb_types { type \"T\" { modifiers = Shift; \
         map[Shift] = 2000000000; }; }; xkb_compat { include \"complete\" }; \
         xkb_symbols { key <A> { type = \"T\", [a, b] }; }; };",
    ] {
        let mut agent_client = Client::connect(agent(&compositor));
        let fd = memfd(bomb.len());
        write_at(&fd, bomb.as_bytes(), 0);
        agent_client.virtual_keyboard().keymap(XKB_V1, fd.as_fd(), bomb.len() as u32);
        if let Err(why) = agent_client.sync() {
            compositor.assert_alive();
            panic!("the agent was disconnected for a keymap it may send: {why}\n{}", compositor.log());
        }
    }
    compositor.assert_alive();
    let grown = compositor.rss_kib().saturating_sub(rss_before);
    assert!(grown < 16 * 1024, "the agent's keymaps grew the compositor by {grown} KiB");

    let mut agent_client = Client::connect(agent(&compositor));
    let keyboard = agent_client.virtual_keyboard();
    keyboard.keymap(XKB_V1, keymap.as_fd(), size);
    keyboard.key(0, 30, 1);
    keyboard.key(1, 30, 0);
    agent_client.sync().expect("a real keymap and its keys are accepted");
    browser.wait(TIMEOUT, |app| app.keys == [(30, true), (30, false)]).expect("the keys reach the browser");
    serves_next(&mut compositor);
}

/// Client geometry at the ends of `i32` (damage, a window geometry offset, a subsurface position, the agent's capture
/// regions) is clamped instead of overflowing: these tests run the debug build, which checks every addition, so an
/// overflow would end the compositor.
#[test]
fn geometry_at_the_ends_of_i32_is_clamped() {
    let mut compositor = Compositor::start_with(Options { agent_uid: Some(current_uid()), ..Options::default() });
    let mut client = Client::connect(&compositor.browser_socket);
    let window = client.window().expect("a toplevel");
    let pool = client.pool(64 * 64 * 4);
    let buffer = client.buffer(&pool, 0, 64, 64, 256);
    let child = client.surface();
    let subsurface = client.subcompositor.get_subsurface(&child, &window.surface, &client.qh, ());
    subsurface.set_position(i32::MAX, i32::MAX);
    child.attach(Some(&buffer), 0, 0);
    child.damage_buffer(i32::MAX, i32::MAX, i32::MAX, i32::MAX);
    child.commit();
    window.xdg_surface.set_window_geometry(i32::MIN, i32::MIN, 64, 64);
    window.surface.attach(Some(&buffer), 0, 0);
    window.surface.damage_buffer(1, 0, i32::MAX, 1);
    window.surface.damage(i32::MAX, i32::MAX, i32::MAX, i32::MAX);
    let token = client.token();
    window.surface.frame(&client.qh, token);
    window.surface.commit();
    if let Err(why) = client.wait(TIMEOUT, |app| app.done.contains(&token)) {
        compositor.assert_alive();
        panic!("the extreme geometry was not composed: {why}");
    }

    let mut agent_client = Client::connect(agent(&compositor));
    for region in [(i32::MAX, 0, 100, 1), (0, i32::MAX, 1, i32::MAX), (i32::MIN, i32::MIN, i32::MAX, i32::MAX)] {
        agent_client.capture_region(region);
    }
    if let Err(why) = agent_client.sync() {
        compositor.assert_alive();
        panic!("the agent's extreme capture regions were not answered: {why}");
    }
    client.sync().expect("the browser stays connected");
    serves_next(&mut compositor);
}

/// A popup positioner's coordinates or sizes past the bound within which smithay can add them up in `i32` disconnect
/// the client before smithay places the popup (these tests run the debug build, which checks every addition); a popup
/// whose positioner is at the bound everywhere is placed, and kept inside the output.
#[test]
fn popup_positioner_geometry_past_its_bound_disconnects() {
    let mut compositor = Compositor::start();
    let past: [fn(&XdgPositioner); 4] = [
        |p| p.set_anchor_rect(i32::MAX, 0, 2, 2),
        |p| p.set_offset(i32::MIN, 0),
        |p| p.set_size(MAX_POSITIONER_EXTENT + 1, 1),
        |p| p.set_anchor_rect(0, 0, 1, MAX_POSITIONER_EXTENT + 1),
    ];
    for set in past {
        hostile(&mut compositor, browser, |c| {
            let window = c.window().expect("a toplevel");
            let positioner = c.wm_base.create_positioner(&c.qh, ());
            positioner.set_size(10, 10);
            positioner.set_anchor_rect(0, 0, 1, 1);
            set(&positioner);
            let surface = c.surface();
            let xdg_surface = c.wm_base.get_xdg_surface(&surface, &c.qh, ());
            xdg_surface.get_popup(Some(&window.xdg_surface), &positioner, &c.qh, ());
        });
    }

    let mut client = Client::connect(&compositor.browser_socket);
    let window = client.window().expect("a toplevel");
    let positioner = client.wm_base.create_positioner(&client.qh, ());
    let max = MAX_POSITIONER_EXTENT;
    positioner.set_size(max, max);
    positioner.set_anchor_rect(-max, max, max, max);
    positioner.set_offset(max, -max);
    positioner.set_constraint_adjustment(xdg_positioner::ConstraintAdjustment::all());
    let surface = client.surface();
    let xdg_surface = client.wm_base.get_xdg_surface(&surface, &client.qh, ());
    let popup = xdg_surface.get_popup(Some(&window.xdg_surface), &positioner, &client.qh, ());
    surface.commit();
    popup.reposition(&positioner, 1);
    if let Err(why) = client.sync() {
        compositor.assert_alive();
        panic!("a positioner at the bound was refused: {why}");
    }
    serves_next(&mut compositor);
}

/// Sends one line on the agent's injection socket and returns the answer.
fn inject(stream: &mut UnixStream, line: &str) -> String {
    stream.write_all(format!("{line}\n").as_bytes()).expect("write to the injection socket");
    let mut answer = Vec::new();
    let mut byte = [0u8; 1];
    while byte[0] != b'\n' {
        stream.read_exact(&mut byte).expect("read the injection answer");
        answer.push(byte[0]);
    }
    String::from_utf8(answer).expect("a text answer").trim_end().to_owned()
}

/// A cursor surface whose resize crosses the client's pixel bound, in the same requests that set it as the cursor and
/// gave it its first pixels: the client is disconnected, and the frame that reads the cursor neither reads the
/// disconnected surface's pixels nor brings the compositor down.
#[test]
fn a_cursor_resized_past_the_pixel_bound_disconnects() {
    let mut compositor = Compositor::start_with(Options { agent_uid: Some(current_uid()), ..Options::default() });
    let (w, h) = OUTPUT;
    let mut client = Client::connect(&compositor.browser_socket);
    let pointer = client.pointer();
    let window = client.window().expect("a toplevel");
    let pool = client.pool((w * h * 4) as usize);
    let whole = client.buffer(&pool, 0, w, h, w * 4);
    let shown = client.present(&window.surface, &whole, w, h);
    client.wait(TIMEOUT, |app| app.done.contains(&shown)).expect("the window is shown");

    // The pointer enters the window, so the client may set its cursor.
    let socket = agent(&compositor).with_file_name("inject");
    let mut injection = UnixStream::connect(socket).expect("the injection socket");
    assert_eq!(inject(&mut injection, "cua-inject v1"), "cua-inject v1");
    assert_eq!(inject(&mut injection, "d 100 100 1 272"), "ok");
    client.wait(TIMEOUT, |app| app.pointer_enter.is_some()).expect("the pointer enters the window");
    let serial = client.app.pointer_enter.expect("an enter serial");

    // Surfaces without a role hold the client's pixel copies up to 32x32 pixels short of its bound: the window, six
    // more outputs, and 320x236 + 16x16 pixels.
    let mut sizes = vec![(w, h); MAX_PIXEL_OUTPUTS as usize - 2];
    sizes.extend([(320, 236), (16, 16)]);
    assert_eq!(sizes.iter().map(|(sw, sh)| sw * sh).sum::<i32>() + w * h + 32 * 32, MAX_PIXEL_OUTPUTS * w * h);
    let mut held = Vec::new();
    for (sw, sh) in sizes {
        let surface = client.surface();
        let buffer = client.buffer(&pool, 0, sw, sh, sw * 4);
        surface.attach(Some(&buffer), 0, 0);
        surface.damage_buffer(0, 0, sw, sh);
        surface.commit();
        held.push((surface, buffer));
    }
    client.sync().expect("pixel copies up to the bound are accepted");

    // In one batch, before any frame: the cursor, its first 32x32 pixels (exactly the bound), then 33x32 (past it).
    let cursor = client.surface();
    let fits = client.buffer(&pool, 0, 32, 32, 32 * 4);
    let past = client.buffer(&pool, 0, 33, 32, 33 * 4);
    pointer.set_cursor(serial, Some(&cursor), 0, 0);
    cursor.attach(Some(&fits), 0, 0);
    cursor.damage_buffer(0, 0, 32, 32);
    cursor.commit();
    cursor.attach(Some(&past), 0, 0);
    cursor.commit();
    let why = client.expect_disconnect(TIMEOUT);
    eprintln!("disconnected: {why}");
    // The frame that follows reads the cursor.
    std::thread::sleep(Duration::from_millis(200));
    serves_next(&mut compositor);
}

/// A surface outside the scene (it has no role, so it is never drawn) that changes size at every commit, or unmaps
/// and maps again: what the compositor keeps of the surface's changes to recompose stays bounded however many come.
#[test]
fn size_changes_of_a_surface_that_is_never_drawn_stay_bounded() {
    let flood = |unmap: bool| {
        list_flood(|c| {
            let surface = c.surface();
            let pool = c.pool(4096);
            let one = c.buffer(&pool, 0, 1, 1, 4).id().protocol_id();
            let two = if unmap { 0 } else { c.buffer(&pool, 0, 2, 1, 8).id().protocol_id() };
            let s = surface.id().protocol_id();
            [request(s, 1, &[one, 0, 0]), request(s, 6, &[]), request(s, 1, &[two, 0, 0]), request(s, 6, &[])].concat()
        })
    };
    let cases = [("resize", flood(false)), ("unmap", flood(true))];
    for (case, (sent, closed, grown)) in cases {
        eprintln!("{case} flood: {sent} pairs of commits, closed {closed}, RSS +{grown} KiB");
        assert!(sent >= 10_000 && !closed, "the {case} flood is valid however much of it comes");
        // Kept per commit, 16 bytes each would be over a megabyte by now.
        assert!(grown < 512, "the {case} flood grew the compositor by {grown} KiB");
    }
}

/// A positioner every popup of these tests uses.
fn popup_positioner(c: &mut Client) -> XdgPositioner {
    let positioner = c.wm_base.create_positioner(&c.qh, ());
    positioner.set_size(10, 10);
    positioner.set_anchor_rect(0, 0, 1, 1);
    positioner
}

/// A popup's chain of parents is checked whenever it is placed: a popup that is its own parent (smithay walks the
/// chain to its root, which a cycle never reaches), whose parent has no role yet (it could become a popup later and
/// lengthen the chain), or whose parent's surface was destroyed (smithay keeps that parent's role and placement, so
/// chains through dead surfaces would grow past every surface bound), disconnects the client. A popup of a popup is
/// placed.
#[test]
fn popups_on_a_dead_or_cyclic_parent_disconnect() {
    let mut compositor = Compositor::start();
    hostile(&mut compositor, browser, |c| {
        let positioner = popup_positioner(c);
        let surface = c.surface();
        let xdg_surface = c.wm_base.get_xdg_surface(&surface, &c.qh, ());
        xdg_surface.get_popup(Some(&xdg_surface), &positioner, &c.qh, ());
    });
    hostile(&mut compositor, browser, |c| {
        let window = c.window().expect("a toplevel");
        let positioner = popup_positioner(c);
        let first = c.surface();
        let first_xdg = c.wm_base.get_xdg_surface(&first, &c.qh, ());
        let _first_popup = first_xdg.get_popup(Some(&window.xdg_surface), &positioner, &c.qh, ());
        first.commit();
        c.sync().expect("the first popup is accepted");
        // Its surface goes, its xdg objects stay; the next popup is created in a later turn.
        first.destroy();
        c.sync().expect("smithay accepts the surface's destruction");
        let second = c.surface();
        let second_xdg = c.wm_base.get_xdg_surface(&second, &c.qh, ());
        second_xdg.get_popup(Some(&first_xdg), &positioner, &c.qh, ());
    });
    hostile(&mut compositor, browser, |c| {
        let positioner = popup_positioner(c);
        let unmade = c.surface();
        let unmade_xdg = c.wm_base.get_xdg_surface(&unmade, &c.qh, ());
        let surface = c.surface();
        let xdg_surface = c.wm_base.get_xdg_surface(&surface, &c.qh, ());
        xdg_surface.get_popup(Some(&unmade_xdg), &positioner, &c.qh, ());
    });
    // Every one came from the check of the chain, not from another.
    let log = compositor.log();
    let reason = "message: \"a popup on a destroyed or unmade parent, or past the popup depth bound\"";
    assert_eq!(log.matches(reason).count(), 3, "{log}");

    let mut client = Client::connect(&compositor.browser_socket);
    let window = client.window().expect("a toplevel");
    let positioner = popup_positioner(&mut client);
    let mut parent = window.xdg_surface.clone();
    let mut popups = Vec::new();
    for _ in 0..3 {
        let surface = client.surface();
        let xdg_surface = client.wm_base.get_xdg_surface(&surface, &client.qh, ());
        popups.push(xdg_surface.get_popup(Some(&parent), &positioner, &client.qh, ()));
        surface.commit();
        parent = xdg_surface;
    }
    if let Err(why) = client.sync() {
        compositor.assert_alive();
        panic!("popups of popups were refused: {why}");
    }
    serves_next(&mut compositor);
}

/// The agent's virtual pointer presses only mouse buttons: smithay tracks every code it is sent as held, before the
/// compositor filters them, so a code outside `BTN_LEFT..=BTN_TASK` disconnects the agent before smithay sees it.
/// Every mouse button is accepted.
#[test]
fn virtual_pointer_buttons_outside_the_mouse_range_disconnect() {
    use wayland_client::protocol::wl_pointer::ButtonState;
    const BTN_LEFT: u32 = 0x110;
    const BTN_TASK: u32 = 0x117;
    let mut compositor = Compositor::start_with(Options { agent_uid: Some(current_uid()), ..Options::default() });
    for (code, state) in [(0, ButtonState::Pressed), (BTN_LEFT - 1, ButtonState::Pressed)]
        .into_iter()
        .chain([(BTN_TASK + 1, ButtonState::Pressed), (u32::MAX, ButtonState::Released)])
    {
        hostile(&mut compositor, agent, |c| c.virtual_pointer().button(0, code, state));
    }
    let mut agent_client = Client::connect(agent(&compositor));
    let pointer = agent_client.virtual_pointer();
    for code in BTN_LEFT..=BTN_TASK {
        pointer.button(0, code, ButtonState::Pressed);
        pointer.button(1, code, ButtonState::Released);
        pointer.frame();
    }
    agent_client.sync().expect("every mouse button is accepted");
    serves_next(&mut compositor);
}

/// The agent's screencopy, flooded with captures of the whole output into one buffer: copies are made at the
/// compositions, one at a time, so a person's client keeps getting answers promptly, and a later capture still gets
/// its copy.
#[test]
fn a_screencopy_flood_keeps_the_compositor_responsive() {
    use wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1;
    const OUTPUT_SIZE: (i32, i32) = (1920, 1080);
    let (w, h) = OUTPUT_SIZE;
    let mut compositor =
        Compositor::start_with(Options { output: "1920x1080", agent_uid: Some(current_uid()), ..Options::default() });
    let mut probe = Probe::new(&compositor);
    let mut flood = Client::connect(agent(&compositor));
    let output = flood.globals.bind::<wl_output::WlOutput, _, _>(&flood.qh, 1..=4, ()).expect("wl_output");
    let manager =
        flood.globals.bind::<ZwlrScreencopyManagerV1, _, _>(&flood.qh, 1..=3, ()).expect("the screencopy manager");
    let pool = flood.pool((w * h * 4) as usize);
    let buffer = flood.buffer(&pool, 0, w, h, w * 4);
    // A frame id the client library frees and then never allocates again: the flood captures, copies, and destroys
    // under it, over and over.
    let first = manager.capture_output(0, &output, &flood.qh, ());
    let frame = first.id().protocol_id();
    first.destroy();
    flood.sync().expect("the flood's objects are created, and the frame id freed");
    let (m, o, b) = (manager.id().protocol_id(), output.id().protocol_id(), buffer.id().protocol_id());
    let copy = [request(m, 0, &[frame, 0, o]), request(frame, 0, &[b]), request(frame, 1, &[])].concat();

    let flooding = Arc::new(AtomicBool::new(true));
    let attacker = {
        let flooding = flooding.clone();
        std::thread::spawn(move || {
            let result = raw_flood(flood.conn.as_fd(), FLOOD, |chunk| chunk.extend(copy.repeat(1024)));
            flooding.store(false, Ordering::SeqCst);
            (result, flood)
        })
    };
    let (syncs, slowest) = probe.syncs_while(&flooding);
    let ((sent, error), flood) = attacker.join().expect("the flood thread");
    drop(flood);
    eprintln!(
        "screencopy flood at {w}x{h}: {} captures in {FLOOD:?} (ended by {error:?}); {syncs} probe syncs, slowest \
         {slowest:?}",
        sent / 40
    );
    assert!(error.is_none(), "the flood was disconnected: {error:?}\n{}", compositor.log());
    assert!(syncs >= 10, "the probe ran during the flood ({syncs} syncs)");
    assert!(slowest < Duration::from_millis(250), "a probe sync waited {slowest:?} during the flood");

    let mut agent_client = Client::connect(agent(&compositor));
    let pool = agent_client.pool((w * h * 4) as usize);
    let buffer = agent_client.buffer(&pool, 0, w, h, w * 4);
    let frame = agent_client.capture_region((0, 0, w, h));
    agent_client.sync().expect("the capture is answered");
    frame.copy(&buffer);
    agent_client.wait(TIMEOUT, |app| app.copies_ready == 1).expect("the copy is made");
    serves_next(&mut compositor);
}

/// A buffer attached again, with more damage, before the frame that reads it is released once, by that frame.
#[test]
fn a_buffer_attached_again_while_pending_is_released_once() {
    let mut compositor = Compositor::start();
    let mut client = Client::connect(&compositor.browser_socket);
    let window = client.window().expect("a toplevel");
    let pool = client.pool(64 * 64 * 4);
    let buffer = client.buffer(&pool, 0, 64, 64, 256);
    let first = client.present(&window.surface, &buffer, 64, 64);
    client.wait(TIMEOUT, |app| app.done.contains(&first)).expect("the first frame");
    client.sync().expect("the first frame's release");
    assert_eq!(client.app.releases, 1, "the first frame released its buffer");
    // Both commits in one batch, so the compositor takes them before its next frame.
    window.surface.attach(Some(&buffer), 0, 0);
    window.surface.damage_buffer(0, 0, 8, 8);
    window.surface.commit();
    let second = client.present(&window.surface, &buffer, 64, 64);
    client.wait(TIMEOUT, |app| app.done.contains(&second)).expect("the second frame");
    client.sync().expect("the second frame's release");
    assert_eq!(client.app.releases, 2, "the buffer attached twice was released once");
    serves_next(&mut compositor);
}
