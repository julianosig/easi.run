//! A small Wayland client for hostile-client tests: it can behave, or send exactly the requests a test chooses.

use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use wayland_client::backend::WaylandError;
use wayland_client::globals::{GlobalList, GlobalListContents, registry_queue_init};
use wayland_client::protocol::{
    wl_buffer::{self, WlBuffer},
    wl_callback::{self, WlCallback},
    wl_compositor::WlCompositor,
    wl_data_device_manager::WlDataDeviceManager,
    wl_data_source::WlDataSource,
    wl_keyboard::{self, WlKeyboard},
    wl_output::WlOutput,
    wl_pointer::{self, WlPointer},
    wl_region::WlRegion,
    wl_registry::WlRegistry,
    wl_seat::WlSeat,
    wl_shm::{self, WlShm},
    wl_shm_pool::WlShmPool,
    wl_subcompositor::WlSubcompositor,
    wl_subsurface::WlSubsurface,
    wl_surface::WlSurface,
};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, delegate_noop, event_created_child};
use wayland_protocols::xdg::shell::client::{
    xdg_popup::{self, XdgPopup},
    xdg_positioner::XdgPositioner,
    xdg_surface::{self, XdgSurface},
    xdg_toplevel::XdgToplevel,
    xdg_wm_base::{self, XdgWmBase},
};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1, zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1,
    zwlr_foreign_toplevel_manager_v1::{self, ZwlrForeignToplevelManagerV1},
};
use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1},
    zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1, zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};

/// What the client has heard from the compositor.
#[derive(Default)]
pub struct App {
    /// Tokens of the callbacks (frame or sync) that are done.
    pub done: Vec<u32>,
    /// The serial of the last `xdg_surface.configure` not yet acknowledged.
    pub configure: Option<u32>,
    /// The serial of the last `wl_pointer.enter`.
    pub pointer_enter: Option<u32>,
    /// The surface the pointer is on (its protocol id), and where on it, from the last `wl_pointer.enter` or `motion`.
    pub pointer_at: Option<(u32, f64, f64)>,
    /// The surface the last `wl_pointer.axis` reached (its protocol id), as `pointer_at` had it then.
    pub axis_on: Option<u32>,
    /// The seat's keymap, from `wl_keyboard.keymap`: its file and size.
    pub keymap: Option<(OwnedFd, u32)>,
    /// The keys `wl_keyboard.key` reported, and whether each was pressed.
    pub keys: Vec<(u32, bool)>,
    /// The surface with the keyboard focus (its protocol id), from the last `wl_keyboard.enter` or `leave`.
    pub keyboard_on: Option<u32>,
    /// `xdg_popup.popup_done` events, of every popup.
    pub popups_done: usize,
    /// `wl_buffer.release` events, of every buffer.
    pub releases: usize,
    /// Screencopy frames answered `ready`.
    pub copies_ready: usize,
    /// The damage screencopy frames reported (x, y, width, height), of every frame.
    pub copies_damage: Vec<(u32, u32, u32, u32)>,
}

impl Dispatch<WlRegistry, GlobalListContents> for App {
    fn event(
        _: &mut App,
        _: &WlRegistry,
        _: <WlRegistry as Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
    }
}

impl Dispatch<WlCallback, u32> for App {
    fn event(
        app: &mut App,
        _: &WlCallback,
        event: wl_callback::Event,
        token: &u32,
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            app.done.push(*token);
        }
    }
}

impl Dispatch<WlPointer, ()> for App {
    fn event(app: &mut App, _: &WlPointer, event: wl_pointer::Event, _: &(), _: &Connection, _: &QueueHandle<App>) {
        match event {
            wl_pointer::Event::Enter { serial, surface, surface_x, surface_y } => {
                app.pointer_enter = Some(serial);
                app.pointer_at = Some((surface.id().protocol_id(), surface_x, surface_y));
            }
            wl_pointer::Event::Motion { surface_x, surface_y, .. } => {
                if let Some((_, x, y)) = &mut app.pointer_at {
                    (*x, *y) = (surface_x, surface_y);
                }
            }
            wl_pointer::Event::Axis { .. } => app.axis_on = app.pointer_at.map(|(surface, _, _)| surface),
            _ => {}
        }
    }
}

impl Dispatch<WlKeyboard, ()> for App {
    fn event(app: &mut App, _: &WlKeyboard, event: wl_keyboard::Event, _: &(), _: &Connection, _: &QueueHandle<App>) {
        match event {
            wl_keyboard::Event::Keymap { fd, size, .. } => app.keymap = Some((fd, size)),
            wl_keyboard::Event::Enter { surface, .. } => app.keyboard_on = Some(surface.id().protocol_id()),
            wl_keyboard::Event::Leave { .. } => app.keyboard_on = None,
            wl_keyboard::Event::Key { key, state, .. } => {
                app.keys.push((key, state == wayland_client::WEnum::Value(wl_keyboard::KeyState::Pressed)))
            }
            _ => {}
        }
    }
}

impl Dispatch<XdgPopup, ()> for App {
    fn event(app: &mut App, _: &XdgPopup, event: xdg_popup::Event, _: &(), _: &Connection, _: &QueueHandle<App>) {
        if let xdg_popup::Event::PopupDone = event {
            app.popups_done += 1;
        }
    }
}

impl Dispatch<ZwlrScreencopyFrameV1, ()> for App {
    fn event(
        app: &mut App,
        _: &ZwlrScreencopyFrameV1,
        event: zwlr_screencopy_frame_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
        match event {
            zwlr_screencopy_frame_v1::Event::Ready { .. } => app.copies_ready += 1,
            zwlr_screencopy_frame_v1::Event::Damage { x, y, width, height } => {
                app.copies_damage.push((x, y, width, height))
            }
            _ => {}
        }
    }
}

impl Dispatch<WlBuffer, ()> for App {
    fn event(app: &mut App, _: &WlBuffer, event: wl_buffer::Event, _: &(), _: &Connection, _: &QueueHandle<App>) {
        if let wl_buffer::Event::Release = event {
            app.releases += 1;
        }
    }
}

impl Dispatch<XdgWmBase, ()> for App {
    fn event(_: &mut App, base: &XdgWmBase, event: xdg_wm_base::Event, _: &(), _: &Connection, _: &QueueHandle<App>) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            base.pong(serial);
        }
    }
}

impl Dispatch<XdgSurface, ()> for App {
    fn event(app: &mut App, _: &XdgSurface, event: xdg_surface::Event, _: &(), _: &Connection, _: &QueueHandle<App>) {
        if let xdg_surface::Event::Configure { serial } = event {
            app.configure = Some(serial);
        }
    }
}

/// The agent's toplevel list: its handles are made and ignored.
impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for App {
    fn event(
        _: &mut App,
        _: &ZwlrForeignToplevelManagerV1,
        _: zwlr_foreign_toplevel_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
    }

    event_created_child!(App, ZwlrForeignToplevelManagerV1, [
        zwlr_foreign_toplevel_manager_v1::EVT_TOPLEVEL_OPCODE => (ZwlrForeignToplevelHandleV1, ()),
    ]);
}

delegate_noop!(App: ignore ZwlrForeignToplevelHandleV1);
delegate_noop!(App: WlCompositor);
delegate_noop!(App: WlSubcompositor);
delegate_noop!(App: WlSubsurface);
delegate_noop!(App: WlShmPool);
delegate_noop!(App: WlRegion);
delegate_noop!(App: WlDataDeviceManager);
delegate_noop!(App: ignore WlDataSource);
delegate_noop!(App: ZwpVirtualKeyboardManagerV1);
delegate_noop!(App: ZwpVirtualKeyboardV1);
delegate_noop!(App: ignore WlOutput);
delegate_noop!(App: ZwlrScreencopyManagerV1);
delegate_noop!(App: ZwlrVirtualPointerManagerV1);
delegate_noop!(App: ZwlrVirtualPointerV1);
delegate_noop!(App: ignore WlShm);
delegate_noop!(App: ignore WlSeat);
delegate_noop!(App: ignore WlSurface);
delegate_noop!(App: ignore XdgToplevel);
delegate_noop!(App: XdgPositioner);

/// An xdg toplevel, its surface and its xdg surface.
pub struct Window {
    pub surface: WlSurface,
    pub xdg_surface: XdgSurface,
    pub toplevel: XdgToplevel,
}

/// A memfd-backed pool.
pub struct Pool {
    pub fd: OwnedFd,
    pub pool: WlShmPool,
}

pub struct Client {
    pub conn: Connection,
    pub queue: EventQueue<App>,
    pub qh: QueueHandle<App>,
    pub app: App,
    pub globals: GlobalList,
    pub compositor: WlCompositor,
    pub shm: WlShm,
    pub subcompositor: WlSubcompositor,
    pub wm_base: XdgWmBase,
    next_token: u32,
}

pub const TIMEOUT: Duration = Duration::from_secs(5);
/// What a wait or a flush fails with when the compositor neither answers nor reads by its deadline: not a disconnect.
const TIMED_OUT: &str = "timed out";

/// Connects to a listening socket, retrying while it is not yet listening.
pub fn connect_socket(path: &Path) -> UnixStream {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        match UnixStream::connect(path) {
            Ok(stream) => return stream,
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            Err(e) => panic!("connect {}: {e}", path.display()),
        }
    }
}

/// Whether the compositor serves a Wayland client on `socket`: a roundtrip is answered, where a refused connection is
/// closed. A connection neither answered nor closed within [`TIMEOUT`] fails the test instead of hanging it.
pub fn served(socket: &Path) -> bool {
    let Ok(conn) = Connection::from_socket(connect_socket(socket)) else { return false };
    let mut queue = conn.new_event_queue::<App>();
    let mut app = App::default();
    conn.display().sync(&queue.handle(), 0);
    if conn.flush().is_err() {
        return false;
    }
    let deadline = Instant::now() + TIMEOUT;
    while !app.done.contains(&0) {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(!left.is_zero(), "the compositor neither served nor refused a client on {}", socket.display());
        if read(&conn, &mut queue, &mut app, left.min(Duration::from_millis(100))).is_err() {
            return false;
        }
    }
    true
}

/// Reads and dispatches what arrives on `conn` for `queue` within `timeout`.
fn read(conn: &Connection, queue: &mut EventQueue<App>, app: &mut App, timeout: Duration) -> Result<(), String> {
    queue.dispatch_pending(app).map_err(|e| format!("{e}"))?;
    let Some(guard) = queue.prepare_read() else {
        queue.dispatch_pending(app).map_err(|e| format!("{e}"))?;
        return Ok(());
    };
    let ready = {
        let fd: BorrowedFd<'_> = guard.connection_fd();
        let mut fds = [PollFd::new(&fd, PollFlags::IN)];
        poll(&mut fds, Some(&timespec(timeout))).map_err(|e| format!("poll: {e}"))? > 0
    };
    if ready {
        match guard.read() {
            Ok(_) => {}
            Err(WaylandError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(failure(conn, e)),
        }
    } else {
        drop(guard);
    }
    queue.dispatch_pending(app).map_err(|e| failure(conn, e))?;
    Ok(())
}

/// Creates a memfd of `size` bytes.
pub fn memfd(size: usize) -> OwnedFd {
    let fd = rustix::fs::memfd_create("hostile", rustix::fs::MemfdFlags::CLOEXEC).expect("memfd_create");
    rustix::fs::ftruncate(&fd, size as u64).expect("ftruncate");
    fd
}

/// Writes `bytes` at `offset` of a file.
pub fn write_at(fd: impl AsFd, mut bytes: &[u8], mut offset: u64) {
    while !bytes.is_empty() {
        let n = rustix::io::pwrite(&fd, bytes, offset).expect("pwrite");
        bytes = &bytes[n..];
        offset += n as u64;
    }
}

impl Client {
    /// Connects and binds the globals a browser uses.
    pub fn connect(path: &Path) -> Client {
        let conn = Connection::from_socket(connect_socket(path)).expect("a Wayland connection");
        let (globals, queue) = registry_queue_init::<App>(&conn).expect("the registry");
        let qh = queue.handle();
        let compositor = globals.bind::<WlCompositor, _, _>(&qh, 1..=6, ()).expect("wl_compositor");
        let shm = globals.bind::<WlShm, _, _>(&qh, 1..=1, ()).expect("wl_shm");
        let subcompositor = globals.bind::<WlSubcompositor, _, _>(&qh, 1..=1, ()).expect("wl_subcompositor");
        let wm_base = globals.bind::<XdgWmBase, _, _>(&qh, 1..=6, ()).expect("xdg_wm_base");
        let mut client = Client {
            conn,
            queue,
            qh,
            app: App::default(),
            globals,
            compositor,
            shm,
            subcompositor,
            wm_base,
            next_token: 0,
        };
        client.sync().expect("the first roundtrip");
        client
    }

    pub fn token(&mut self) -> u32 {
        self.next_token += 1;
        self.next_token
    }

    /// Flushes, waiting while the socket is full.
    pub fn flush(&mut self) -> Result<(), String> {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            match self.conn.flush() {
                Ok(()) => return Ok(()),
                Err(WaylandError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() > deadline {
                        return Err(TIMED_OUT.into());
                    }
                    // Read while waiting so the compositor never blocks on our events either.
                    self.read_events(Duration::ZERO)?;
                    let fd = self.conn.as_fd();
                    let mut fds = [PollFd::new(&fd, PollFlags::OUT)];
                    let _ = poll(&mut fds, Some(&timespec(Duration::from_millis(50))));
                }
                Err(e) => return Err(failure(&self.conn, e)),
            }
        }
    }

    /// Reads and dispatches what arrives within `timeout`.
    pub fn read_events(&mut self, timeout: Duration) -> Result<(), String> {
        read(&self.conn, &mut self.queue, &mut self.app, timeout)
    }

    /// Waits until `done` holds, or fails on a disconnect or after `timeout`.
    pub fn wait(&mut self, timeout: Duration, mut done: impl FnMut(&App) -> bool) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        self.flush()?;
        while !done(&self.app) {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(TIMED_OUT.into());
            }
            self.read_events(left.min(Duration::from_millis(100)))?;
        }
        Ok(())
    }

    /// A roundtrip that fails, instead of blocking, when the compositor does not answer.
    pub fn sync(&mut self) -> Result<(), String> {
        let token = self.token();
        self.conn.display().sync(&self.qh, token);
        self.wait(TIMEOUT, |app| app.done.contains(&token))
    }

    /// Asserts that the compositor disconnects this client within `timeout`, and returns why.
    pub fn expect_disconnect(&mut self, timeout: Duration) -> String {
        let deadline = Instant::now() + timeout;
        loop {
            if let Err(why) = self.sync() {
                assert_ne!(why, TIMED_OUT, "the compositor neither answered nor disconnected the client");
                // A failed write can surface before the error event that explains it; read what is left.
                for _ in 0..10 {
                    if self.conn.protocol_error().is_some() || self.read_events(Duration::from_millis(50)).is_err() {
                        break;
                    }
                }
                return failure(&self.conn, why);
            }
            assert!(Instant::now() < deadline, "the compositor kept a hostile client connected");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    pub fn pool(&mut self, size: usize) -> Pool {
        let fd = memfd(size);
        let pool = self.shm.create_pool(fd.as_fd(), size as i32, &self.qh, ());
        Pool { fd, pool }
    }

    pub fn buffer(&mut self, pool: &Pool, offset: i32, width: i32, height: i32, stride: i32) -> WlBuffer {
        pool.pool.create_buffer(offset, width, height, stride, wl_shm::Format::Xrgb8888, &self.qh, ())
    }

    /// A new binding of the seat.
    pub fn seat(&self) -> WlSeat {
        self.globals.bind::<WlSeat, _, _>(&self.qh, 1..=7, ()).expect("wl_seat")
    }

    /// The seat's pointer.
    pub fn pointer(&mut self) -> WlPointer {
        self.seat().get_pointer(&self.qh, ())
    }

    /// The seat's keyboard.
    pub fn keyboard(&mut self) -> WlKeyboard {
        self.seat().get_keyboard(&self.qh, ())
    }

    /// The output and a screencopy manager of at least `min_version` (the agent's socket offers it).
    pub fn screencopy(&mut self, min_version: u32) -> (WlOutput, ZwlrScreencopyManagerV1) {
        let output = self.globals.bind::<WlOutput, _, _>(&self.qh, 1..=4, ()).expect("wl_output");
        let manager = self
            .globals
            .bind::<ZwlrScreencopyManagerV1, _, _>(&self.qh, min_version..=3, ())
            .expect("zwlr_screencopy_manager_v1");
        (output, manager)
    }

    /// Asks the agent's screencopy for a region of the output.
    pub fn capture_region(&mut self, (x, y, width, height): (i32, i32, i32, i32)) -> ZwlrScreencopyFrameV1 {
        let (output, manager) = self.screencopy(1);
        manager.capture_output_region(0, &output, x, y, width, height, &self.qh, ())
    }

    /// A virtual keyboard on the seat (the agent's socket offers the manager).
    pub fn virtual_keyboard(&mut self) -> ZwpVirtualKeyboardV1 {
        let seat = self.seat();
        let manager = self
            .globals
            .bind::<ZwpVirtualKeyboardManagerV1, _, _>(&self.qh, 1..=1, ())
            .expect("zwp_virtual_keyboard_manager_v1");
        manager.create_virtual_keyboard(&seat, &self.qh, ())
    }

    /// A virtual pointer on the seat (the agent's socket offers the manager).
    pub fn virtual_pointer(&mut self) -> ZwlrVirtualPointerV1 {
        let seat = self.seat();
        let manager = self
            .globals
            .bind::<ZwlrVirtualPointerManagerV1, _, _>(&self.qh, 1..=2, ())
            .expect("zwlr_virtual_pointer_manager_v1");
        manager.create_virtual_pointer(Some(&seat), &self.qh, ())
    }

    pub fn surface(&mut self) -> WlSurface {
        self.compositor.create_surface(&self.qh, ())
    }

    /// Creates a toplevel and acknowledges its first configure.
    pub fn window(&mut self) -> Result<Window, String> {
        let surface = self.surface();
        let xdg_surface = self.wm_base.get_xdg_surface(&surface, &self.qh, ());
        let toplevel = xdg_surface.get_toplevel(&self.qh, ());
        toplevel.set_title("hostile-test".into());
        surface.commit();
        self.ack_configure(&xdg_surface)?;
        Ok(Window { surface, xdg_surface, toplevel })
    }

    /// Waits for `xdg_surface`'s next configure and acknowledges it.
    pub fn ack_configure(&mut self, xdg_surface: &XdgSurface) -> Result<(), String> {
        self.wait(TIMEOUT, |app| app.configure.is_some())?;
        xdg_surface.ack_configure(self.app.configure.take().expect("configured"));
        Ok(())
    }

    /// A toplevel ([`Client::window`]) and a `width` x `height` buffer for it, in a pool of its own.
    pub fn window_with_buffer(&mut self, width: i32, height: i32) -> (Window, Pool, WlBuffer) {
        let window = self.window().expect("a toplevel");
        let pool = self.pool((width * height * 4) as usize);
        let buffer = self.buffer(&pool, 0, width, height, width * 4);
        (window, pool, buffer)
    }

    /// Attaches `buffer`, damages it whole, asks for a frame callback, and commits. Returns the callback's token.
    pub fn present(&mut self, surface: &WlSurface, buffer: &WlBuffer, width: i32, height: i32) -> u32 {
        let token = self.token();
        surface.attach(Some(buffer), 0, 0);
        surface.damage_buffer(0, 0, width, height);
        surface.frame(&self.qh, token);
        surface.commit();
        token
    }

    /// [`Client::present`]s `buffer` and waits for its frame callback.
    pub fn show(&mut self, surface: &WlSurface, buffer: &WlBuffer, width: i32, height: i32) -> Result<(), String> {
        let token = self.present(surface, buffer, width, height);
        self.wait(TIMEOUT, |app| app.done.contains(&token))
    }
}

/// Why `conn` failed: the compositor's protocol error when it sent one, otherwise `error`.
fn failure(conn: &Connection, error: impl std::fmt::Display) -> String {
    match conn.protocol_error() {
        Some(p) => format!("protocol error on {}: {}", p.object_interface, p.message),
        None => error.to_string(),
    }
}

fn timespec(d: Duration) -> Timespec {
    Timespec { tv_sec: d.as_secs() as i64, tv_nsec: d.subsec_nanos() as i64 }
}

/// Proves the compositor serves a well-behaved client: it maps a toplevel with real pixels and gets a frame.
pub fn assert_serves(socket: &Path) {
    let mut client = Client::connect(socket);
    let (window, pool, buffer) = client.window_with_buffer(64, 48);
    write_at(&pool.fd, &[0x80; 64 * 48 * 4], 0);
    client
        .show(&window.surface, &buffer, 64, 48)
        .unwrap_or_else(|why| panic!("the well-behaved client got no frame callback: {why}"));
    client.sync().expect("the well-behaved client stays connected");
}

/// Sends raw bytes with `fds` attached in one `sendmsg`, outside any Wayland library.
pub fn send_with_fds(stream: &UnixStream, bytes: &[u8], fds: &[BorrowedFd<'_>]) -> std::io::Result<usize> {
    use rustix::net::{SendAncillaryBuffer, SendAncillaryMessage, SendFlags, sendmsg};
    let mut space = [std::mem::MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(28))];
    let mut control = SendAncillaryBuffer::new(&mut space);
    assert!(control.push(SendAncillaryMessage::ScmRights(fds)), "at most 28 descriptors per message");
    Ok(sendmsg(stream, &[std::io::IoSlice::new(bytes)], &mut control, SendFlags::NOSIGNAL)?)
}

/// One request in wire form: object, opcode, and 32-bit arguments.
pub fn request(object: u32, opcode: u16, args: &[u32]) -> Vec<u8> {
    let size = 8 + 4 * args.len() as u32;
    let mut bytes = Vec::with_capacity(size as usize);
    bytes.extend_from_slice(&object.to_ne_bytes());
    bytes.extend_from_slice(&((size << 16) | opcode as u32).to_ne_bytes());
    for arg in args {
        bytes.extend_from_slice(&arg.to_ne_bytes());
    }
    bytes
}

/// One request in wire form with a single string argument (length with its NUL, then the bytes padded to 4).
pub fn string_request(object: u32, opcode: u16, text: &str) -> Vec<u8> {
    let mut words = vec![text.len() as u32 + 1];
    let mut bytes = text.as_bytes().to_vec();
    bytes.resize((text.len() + 1).next_multiple_of(4), 0);
    words.extend(bytes.chunks(4).map(|c| u32::from_ne_bytes(c.try_into().expect("four bytes"))));
    request(object, opcode, &words)
}

/// `wl_display.sync` creating callback `id`.
pub fn sync_request(id: u32) -> Vec<u8> {
    request(1, 0, &[id])
}

/// `wl_surface.attach(buffer)`, `damage_buffer` of the whole buffer, and `commit`.
pub fn commit_requests(surface: &WlSurface, buffer: &WlBuffer, (width, height): (i32, i32)) -> Vec<u8> {
    let (s, b) = (surface.id().protocol_id(), buffer.id().protocol_id());
    let mut bytes = request(s, 1, &[b, 0, 0]);
    bytes.extend(request(s, 9, &[0, 0, width as u32, height as u32]));
    bytes.extend(request(s, 6, &[]));
    bytes
}

/// Writes the chunks `fill` produces straight onto a Wayland socket, as fast as the compositor takes them, for
/// `duration` (the last chunk possibly in part, so the connection is not for further use), draining and discarding what
/// the compositor sends back. Returns the bytes sent and, when the compositor closed the connection, why.
pub fn raw_flood(
    fd: BorrowedFd<'_>,
    duration: Duration,
    mut fill: impl FnMut(&mut Vec<u8>),
) -> (usize, Option<std::io::Error>) {
    use rustix::net::{RecvFlags, SendFlags, recv, send};
    let mut sink = vec![0u8; 64 * 1024];
    let mut chunk = Vec::new();
    let (mut sent, mut offset) = (0, 0);
    let start = Instant::now();
    loop {
        // Checked before every send, so a compositor that stops reading mid-chunk ends the flood too.
        if start.elapsed() >= duration {
            return (sent, None);
        }
        if offset == chunk.len() {
            chunk.clear();
            fill(&mut chunk);
            offset = 0;
        }
        match send(fd, &chunk[offset..], SendFlags::DONTWAIT | SendFlags::NOSIGNAL) {
            Ok(n) => {
                sent += n;
                offset += n;
            }
            Err(rustix::io::Errno::AGAIN) => {
                let mut fds = [PollFd::new(&fd, PollFlags::OUT | PollFlags::IN)];
                let _ = poll(&mut fds, Some(&timespec(Duration::from_millis(10))));
            }
            Err(e) => return (sent, Some(e.into())),
        }
        while let Ok((n, _)) = recv(fd, &mut sink, RecvFlags::DONTWAIT) {
            if n == 0 {
                break;
            }
        }
    }
}
