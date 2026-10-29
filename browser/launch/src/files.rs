//! The browser's side of `easi_files_v1` (protocol/wayland/easi-files-v1.xml): downloads stream to the display
//! as Chromium writes them (see `downloads`), and files the person uploads arrive on pipes and are stored in the
//! launcher's private `/uploads`, each in its own directory, created exclusively and never through a link.
//!
//! The launcher binds the global before it starts Chromium, so Chromium cannot (the display grants it once), and its
//! watch on /downloads exists before Chromium starts. Losing the display ends the container, as losing any other
//! process the launcher supervises does.

use std::io::{Read, Write};
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::sync::Arc;

use rustix::fs::{Mode, OFlags};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};

use crate::downloads::Downloads;
use crate::gate::Gate;
use crate::portal::Shared;

pub mod protocol {
    #![allow(missing_docs, clippy::all, unused_imports, non_upper_case_globals, non_camel_case_types)]
    use wayland_client;
    use wayland_client::protocol::*;
    pub mod __interfaces {
        use wayland_client::backend as wayland_backend;
        wayland_scanner::generate_interfaces!("../../protocol/wayland/easi-files-v1.xml");
    }
    use self::__interfaces::*;
    wayland_scanner::generate_client_code!("../../protocol/wayland/easi-files-v1.xml");
}

use protocol::easi_files_v1::{self, EasiFilesV1};

/// The launcher's bound global and its Wayland connection.
pub struct Link {
    pub files: EasiFilesV1,
    pub conn: Connection,
    queue: EventQueue<Heard>,
}

/// What handles the display's events: the portal's shared state and the downloads, once they run.
#[derive(Default)]
pub struct Heard {
    shared: Option<Arc<Shared>>,
    downloads: Option<Downloads>,
    pages: Option<Arc<crate::cdp::Pages>>,
}

impl Dispatch<WlRegistry, GlobalListContents> for Heard {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: wayland_client::protocol::wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<EasiFilesV1, ()> for Heard {
    fn event(
        heard: &mut Self,
        files: &EasiFilesV1,
        event: easi_files_v1::Event,
        _: &(),
        conn: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            easi_files_v1::Event::Upload { upload, data, name, size_hi, size_lo } => {
                let size = u64::from(size_hi) << 32 | u64::from(size_lo);
                let (answer, conn) = (files.clone(), conn.clone());
                // One thread per upload (the display allows only a few at once), so a slow one never holds the rest.
                let spawned = std::thread::Builder::new().name("upload".into()).spawn(move || {
                    match store(upload, data, &name, size) {
                        Some(path) => answer.stored(upload, path),
                        None => answer.failed(upload),
                    }
                    let _ = conn.flush();
                });
                if spawned.is_err() {
                    files.failed(upload);
                }
            }
            easi_files_v1::Event::Chosen { chooser, path } => {
                if let Some(shared) = &heard.shared {
                    shared.chosen(chooser, path);
                }
            }
            easi_files_v1::Event::ChooserDone { chooser } => {
                if let Some(shared) = &heard.shared {
                    shared.finish(chooser, true);
                }
            }
            easi_files_v1::Event::ChooserCancelled { chooser } => {
                if let Some(shared) = &heard.shared {
                    shared.finish(chooser, false);
                }
            }
            easi_files_v1::Event::Reading { id } => {
                if let Some(downloads) = &mut heard.downloads {
                    downloads.reading(id);
                }
            }
            easi_files_v1::Event::OpenPage { request, url } => match &heard.pages {
                Some(pages) => pages.open(request, &url),
                None => files.page_opened(request, 0),
            },
            easi_files_v1::Event::StopDownload { id } => {
                if let Some(downloads) = &mut heard.downloads {
                    downloads.stop(id);
                }
            }
        }
    }
}

/// Stores an upload from its pipe as `/uploads/<upload>/<name>`, created exclusively; the path once all `size` bytes,
/// and no more, arrived, else nothing (and nothing left behind).
fn store(upload: u32, data: OwnedFd, name: &str, size: u64) -> Option<String> {
    if size > display_protocol::MAX_UPLOAD {
        return None;
    }
    let dir = format!("{}/{upload}", display_protocol::browser::UPLOAD_DIR);
    rustix::fs::mkdir(&dir, Mode::from_raw_mode(0o700)).ok()?;
    let name: String = name.chars().map(|c| if c == '/' || c.is_control() { '_' } else { c }).collect();
    let name = if name.is_empty() || name == "." || name == ".." { "upload".to_owned() } else { name };
    let path = format!("{dir}/{name}");
    let stored = crate::create_new(&path).ok().and_then(|file| {
        let mut to = std::fs::File::from(file);
        let copied = std::io::copy(&mut std::fs::File::from(data).take(size + 1), &mut to).ok()?;
        to.flush().ok()?;
        (copied == size).then_some(())
    });
    if stored.is_none() {
        let _ = rustix::fs::unlink(&path);
        let _ = rustix::fs::rmdir(&dir);
        return None;
    }
    Some(path)
}

/// Connects to the display's browser socket and binds the global: before Chromium starts, so only the launcher holds
/// it.
pub fn bind(wayland: &str) -> Result<Link, String> {
    let socket = UnixStream::connect(wayland).map_err(|e| format!("{wayland}: {e}"))?;
    let conn = Connection::from_socket(socket).map_err(|e| format!("wayland: {e}"))?;
    let (globals, mut queue) = registry_queue_init::<Heard>(&conn).map_err(|e| format!("wayland registry: {e}"))?;
    let version = EasiFilesV1::interface().version;
    let files: EasiFilesV1 =
        globals.bind(&queue.handle(), version..=version, ()).map_err(|e| format!("easi_files_v1: {e}"))?;
    queue.roundtrip(&mut Heard::default()).map_err(|e| format!("wayland: {e}"))?;
    Ok(Link { files, conn, queue })
}

/// Watches `dir` for downloads and streams each to the display, until either side fails; then the process ends.
/// The watch exists when this returns, before Chromium starts, so no file is missed.
pub fn watch(
    mut link: Link,
    dir: &'static str,
    shared: Arc<Shared>,
    gate: Arc<Gate>,
    sizes: Arc<crate::cdp::Sizes>,
    pages: Arc<crate::cdp::Pages>,
) {
    let watched = watching(dir).unwrap_or_else(|why| {
        eprintln!("browser-launch: files: {why}");
        std::process::exit(1);
    });
    let downloads = Downloads::new(watched, link.files.clone(), gate, shared.clone(), sizes);
    let spawned = std::thread::Builder::new().name("files".into()).spawn(move || {
        if let Err(why) = run(&mut link, Heard { shared: Some(shared), downloads: Some(downloads), pages: Some(pages) })
        {
            eprintln!("browser-launch: files: {why}");
        }
        std::process::exit(1);
    });
    if let Err(e) = spawned {
        eprintln!("browser-launch: files: {e}");
        std::process::exit(1);
    }
}

/// An inotify descriptor told of every file created, moved, opened, closed after writing, or removed in `dir` (a removal
/// only wakes the loop, to let go of a file nothing else would wake it for), `dir` itself, and the launcher's own
/// unwatched directory inside it.
fn watching(dir: &str) -> Result<(OwnedFd, OwnedFd, OwnedFd), String> {
    use rustix::fs::inotify::{self, CreateFlags, WatchFlags};
    let notify = inotify::init(CreateFlags::CLOEXEC | CreateFlags::NONBLOCK).map_err(|e| format!("inotify: {e}"))?;
    let mask = WatchFlags::CREATE
        | WatchFlags::MOVED_FROM
        | WatchFlags::MOVED_TO
        | WatchFlags::OPEN
        | WatchFlags::CLOSE_WRITE
        | WatchFlags::DELETE;
    inotify::add_watch(&notify, dir, mask).map_err(|e| format!("{dir}: {e}"))?;
    let dir_fd = rustix::fs::open(dir, OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC, Mode::empty())
        .map_err(|e| format!("{dir}: {e}"))?;
    let aside = ".launcher";
    rustix::fs::mkdirat(&dir_fd, aside, Mode::from_raw_mode(0o700)).map_err(|e| format!("{dir}/{aside}: {e}"))?;
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let aside = rustix::fs::openat(&dir_fd, aside, flags, Mode::empty()).map_err(|e| format!("{dir}/{aside}: {e}"))?;
    Ok((notify, dir_fd, aside))
}

fn run(link: &mut Link, mut heard: Heard) -> Result<(), String> {
    let Link { conn, queue, .. } = link;
    use rustix::event::{PollFd, PollFlags, Timespec, poll};
    loop {
        conn.flush().map_err(|e| format!("wayland: {e}"))?;
        let guard = queue.prepare_read().ok_or("wayland: events were left unread")?;
        let downloads = heard.downloads.as_ref().ok_or("the downloads are not set up")?;
        let wayland = {
            let connection_fd = guard.connection_fd();
            let blocked = downloads.blocked_sockets();
            let mut fds =
                vec![PollFd::new(downloads.notify(), PollFlags::IN), PollFd::new(&connection_fd, PollFlags::IN)];
            fds.extend(blocked.iter().map(|socket| PollFd::new(*socket, PollFlags::OUT)));
            let timeout = downloads.timeout().map(|t| Timespec { tv_sec: 0, tv_nsec: t.as_nanos() as i64 });
            match poll(&mut fds, timeout.as_ref()) {
                Ok(_) | Err(rustix::io::Errno::INTR) => {}
                Err(e) => return Err(format!("poll: {e}")),
            }
            fds[1].revents()
        };
        if wayland.intersects(PollFlags::IN | PollFlags::ERR | PollFlags::HUP) {
            guard.read().map_err(|e| format!("wayland: {e}"))?;
            queue.dispatch_pending(&mut heard).map_err(|e| format!("wayland: {e}"))?;
        } else {
            drop(guard);
        }
        let downloads = heard.downloads.as_mut().ok_or("the downloads are not set up")?;
        downloads.events()?;
        downloads.tick()?;
    }
}
