//! Compositor state, client identities, and the Smithay protocol handlers.

use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use calloop::LoopHandle;
use smithay::backend::input::InputEvent;
use smithay::desktop::{PopupKind, PopupManager};
use smithay::input::pointer::CursorImageStatus;
use smithay::input::{Seat, SeatHandler, SeatState};
use smithay::output::Output;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{SERIAL_COUNTER, Serial};
use smithay::wayland::buffer::BufferHandler;
use smithay::wayland::compositor::{Cacheable, CompositorClientState, CompositorHandler, CompositorState, with_states};
use smithay::wayland::foreign_toplevel_list::{
    ForeignToplevelHandle, ForeignToplevelListHandler, ForeignToplevelListState,
};
use smithay::wayland::output::OutputHandler;
use smithay::wayland::selection::SelectionHandler;
use smithay::wayland::selection::data_device::{DataDeviceHandler, DataDeviceState, WaylandDndGrabHandler};
use smithay::wayland::shell::xdg::{
    PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState, XdgToplevelSurfaceData,
};
use smithay::wayland::virtual_pointer::{VirtualPointerBackend, VirtualPointerHandler};
use wayland_server::backend::{ClientData, ClientId, DisconnectReason};
use wayland_server::protocol::{wl_buffer, wl_seat};
use wayland_server::{Client, DisplayHandle, Resource};

use crate::compose::Scene;
use crate::limits;

static OUTPUT_SIZE: OnceLock<(u32, u32)> = OnceLock::new();

/// The output size, fixed for the life of the process.
pub fn output_size() -> (u32, u32) {
    *OUTPUT_SIZE.get().expect("the output size is set before any client connects")
}

pub fn set_output_size(width: u32, height: u32) {
    OUTPUT_SIZE.set((width, height)).expect("the output size is set once");
}

/// Which listening socket a client came through. The browser socket offers only what an ordinary application
/// needs; the privileged socket adds window listing, capture, and input injection for the automation agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientKind {
    Browser,
    Agent,
}

/// Live object counts of one client, shared with the objects that hold them.
#[derive(Default, Debug)]
pub struct Accounting {
    pub pools: AtomicUsize,
    pub pool_bytes: AtomicUsize,
    /// Pool descriptors the compositor holds, including those of destroyed pools that buffers still use.
    pub pool_fds: AtomicUsize,
    pub buffers: AtomicUsize,
    pub surfaces: AtomicUsize,
    pub subsurfaces: AtomicUsize,
    /// Bytes of the compositor's own copies of the client's surfaces (see [`crate::compose`]).
    pub pixel_bytes: AtomicUsize,
}

pub struct ClientState {
    pub compositor: CompositorClientState,
    pub kind: ClientKind,
    pub accounting: Arc<Accounting>,
}

/// Live clients of each socket, so one socket cannot take every descriptor the compositor has.
static BROWSER_CLIENTS: AtomicUsize = AtomicUsize::new(0);
static AGENT_CLIENTS: AtomicUsize = AtomicUsize::new(0);

fn live_clients(kind: ClientKind) -> &'static AtomicUsize {
    match kind {
        ClientKind::Browser => &BROWSER_CLIENTS,
        ClientKind::Agent => &AGENT_CLIENTS,
    }
}

impl ClientState {
    /// A new client's state, or `None` when its socket already serves its limit of clients.
    pub fn new(kind: ClientKind) -> Option<ClientState> {
        let limit = match kind {
            ClientKind::Browser => limits::MAX_BROWSER_CLIENTS,
            ClientKind::Agent => limits::MAX_AGENT_CLIENTS,
        };
        let live = live_clients(kind);
        if live.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= limit {
            live.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
            return None;
        }
        Some(ClientState { compositor: CompositorClientState::default(), kind, accounting: Arc::default() })
    }
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, client_id: ClientId, reason: DisconnectReason) {
        live_clients(self.kind).fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        tracing::info!(?client_id, ?reason, kind = ?self.kind, "client disconnected");
    }
}

pub fn client_kind(client: &Client) -> Option<ClientKind> {
    client.get_data::<ClientState>().map(|state| state.kind)
}

/// The event loop's shared data: the state and the display it dispatches.
pub struct Data {
    pub state: State,
    pub display: wayland_server::Display<State>,
}

/// The browser's one toplevel, fullscreen at the output's origin.
pub struct Toplevel {
    pub surface: ToplevelSurface,
    pub handle: Option<ForeignToplevelHandle>,
    pub wlr_handles: Vec<wayland_protocols_wlr::foreign_toplevel::v1::server::zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1>,
    pub title: String,
    pub app_id: String,
}

pub struct State {
    pub start: Instant,
    pub dh: DisplayHandle,
    pub handle: LoopHandle<'static, Data>,
    pub compositor: CompositorState,
    pub xdg_shell: XdgShellState,
    pub seat_state: SeatState<State>,
    pub data_device: DataDeviceState,
    pub foreign_toplevels: ForeignToplevelListState,
    pub popups: PopupManager,
    pub seat: Seat<State>,
    pub output: Output,
    pub toplevels: Vec<Toplevel>,
    pub wlr_managers:
        Vec<wayland_protocols_wlr::foreign_toplevel::v1::server::zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1>,
    pub scene: Scene,
    pub cursor: CursorImageStatus,
    /// The icon of a drag the browser started (what it drags, under the pointer), drawn at the pointer.
    pub drag_icon: Option<DragIcon>,
    /// The pointer buttons held down, by whoever holds control.
    pub buttons: Vec<u32>,
    /// The agent's virtual keyboards that pressed a key (their sources' ids), so a handoff releases what they hold.
    pub agent_keyboards: std::collections::HashSet<u64>,
    pub lease: crate::input::Lease,
    pub typing: crate::input::Typing,
    /// The browser's copy being read for the person holding control.
    pub copy: crate::copy::Job,
    /// Downloads kept for the person (see [`crate::files`]).
    pub files: crate::files::Files,
    /// The holder's uploads and the page's open file chooser (see [`crate::uploads`]).
    pub uploads: crate::uploads::Uploads,
    /// The PID the agent first named for a browser toplevel, and that toplevel (see [`crate::inject`]).
    pub inject_binding: Option<(u32, WlSurface)>,
    pub chars: crate::inject::CharTable,
    pub hub: crate::stream::Hub,
    /// The recording under way, if any.
    pub recordings: crate::recording::Recordings,
    /// The agent's screen copies waiting for the next composition.
    pub copies: crate::screencopy::Copies,
    /// The shortest time between two compositions (DISPLAY_COMPOSE_GAP_MS).
    pub compose_gap: std::time::Duration,
    pub last_frame: Instant,
    /// Counts compositions for presentation feedback.
    pub presentation_seq: u64,
}

impl State {
    pub fn now_ms(&self) -> u32 {
        self.start.elapsed().as_millis() as u32
    }

    pub fn next_serial(&self) -> Serial {
        SERIAL_COUNTER.next_serial()
    }

    /// The browser toplevel the agent and the person address: the top mapped one, the one shown.
    pub fn browser_toplevel(&self) -> Option<&Toplevel> {
        self.toplevels.iter().rev().find(|t| self.scene.is_mapped(t.surface.wl_surface()))
    }

    /// Disconnects the client of `surface`, telling it `why`.
    pub fn kill_client_of(&self, surface: &WlSurface, why: &str) {
        if let Ok(client) = self.dh.get_client(surface.id()) {
            tracing::warn!(why, "disconnecting a client");
            client.kill(
                &self.dh,
                wayland_server::backend::protocol::ProtocolError {
                    code: 0,
                    object_id: surface.id().protocol_id(),
                    object_interface: "wl_surface".into(),
                    message: why.into(),
                },
            );
        }
    }
}

impl BufferHandler for State {
    fn buffer_destroyed(&mut self, _buffer: &wl_buffer::WlBuffer) {}
}

/// Marks a surface already counted against [`limits::MAX_SUBSURFACES`].
struct CountedSubsurface;

impl CompositorHandler for State {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client.get_data::<ClientState>().expect("every client is inserted with its state").compositor
    }

    fn new_surface(&mut self, surface: &WlSurface) {
        let Ok(client) = self.dh.get_client(surface.id()) else { return };
        let accounting = client.get_data::<ClientState>().expect("client state").accounting.clone();
        let count = accounting.surfaces.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        smithay::wayland::compositor::add_destruction_hook(surface, move |_: &mut State, _surface| {
            accounting.surfaces.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        });
        // Before any commit, so every snapshot of the surface's state is counted.
        with_states(surface, |states| drop(states.cached_state.get::<Snapshots>()));
        smithay::wayland::compositor::add_pre_commit_hook(surface, bound_held_snapshots);
        if count > limits::MAX_SURFACES {
            self.kill_client_of(surface, "surface limit exceeded");
        }
    }

    fn new_subsurface(&mut self, surface: &WlSurface, _parent: &WlSurface) {
        // A browser makes the same surface a subsurface again each time it shows it: count each surface once.
        if !with_states(surface, |states| states.data_map.insert_if_missing(|| CountedSubsurface)) {
            return;
        }
        let Ok(client) = self.dh.get_client(surface.id()) else { return };
        let accounting = client.get_data::<ClientState>().expect("client state").accounting.clone();
        let count = accounting.subsurfaces.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        smithay::wayland::compositor::add_destruction_hook(surface, move |_: &mut State, _surface| {
            accounting.subsurfaces.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        });
        if count > limits::MAX_SUBSURFACES {
            self.kill_client_of(surface, "subsurface limit exceeded");
        }
    }

    fn commit(&mut self, surface: &WlSurface) {
        if let Err(why) = self.scene.commit(surface) {
            self.kill_client_of(surface, why);
            return;
        }
        crate::shell::handle_commit(self, surface);
        if let Some(icon) = self.drag_icon.as_mut().filter(|icon| &icon.surface == surface) {
            with_states(surface, |states| {
                let mut attributes = states.cached_state.get::<smithay::wayland::compositor::SurfaceAttributes>();
                let delta = attributes.current().buffer_delta.take().unwrap_or_default();
                icon.offset = (icon.offset.x.saturating_add(delta.x), icon.offset.y.saturating_add(delta.y)).into();
            });
        }
        self.schedule_frame();
    }
}

/// Counts the snapshots smithay holds of a surface's state, as part of that state: smithay snapshots the pending state
/// at every commit (and again when a parent's commit takes a synchronized child's) and keeps each snapshot until it
/// applies it, which for a synchronized subsurface waits for its parent's commit. Each snapshot taken counts in the
/// pending state's `taken`, each applied in the current state's `applied`; the difference is what smithay holds.
#[derive(Default)]
struct Snapshots {
    taken: usize,
    applied: usize,
}

impl Cacheable for Snapshots {
    fn commit(&mut self, _dh: &DisplayHandle) -> Self {
        self.taken += 1;
        Snapshots { taken: 0, applied: 1 }
    }

    fn merge_into(self, into: &mut Self, _dh: &DisplayHandle) {
        into.applied += self.applied;
    }
}

/// Disconnects a client whose commit would leave smithay holding more than
/// [`limits::MAX_PENDING_SUBSURFACE_COMMITS`] snapshots of one surface.
fn bound_held_snapshots(state: &mut State, _dh: &DisplayHandle, surface: &WlSurface) {
    let held = with_states(surface, |states| {
        let mut snapshots = states.cached_state.get::<Snapshots>();
        snapshots.pending().taken - snapshots.current().applied
    });
    if held >= limits::MAX_PENDING_SUBSURFACE_COMMITS {
        state.kill_client_of(surface, "too many subsurface commits pending their parent's");
    }
}

impl smithay::wayland::shell::xdg::decoration::XdgDecorationHandler for State {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        server_side(&toplevel);
    }

    fn request_mode(
        &mut self,
        toplevel: ToplevelSurface,
        _mode: smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode,
    ) {
        server_side(&toplevel);
    }

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        server_side(&toplevel);
    }
}

/// Every window is server-side decorated, with nothing drawn: maximized windows need no frame.
fn server_side(toplevel: &ToplevelSurface) {
    use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode;
    toplevel.with_pending_state(|s| s.decoration_mode = Some(Mode::ServerSide));
    if toplevel.is_initial_configure_sent() {
        toplevel.send_configure();
    }
}

impl XdgShellHandler for State {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        crate::shell::new_toplevel(self, surface);
    }

    fn fullscreen_request(
        &mut self,
        surface: ToplevelSurface,
        _output: Option<smithay::reexports::wayland_server::protocol::wl_output::WlOutput>,
    ) {
        crate::shell::set_fullscreen(&surface, true);
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        crate::shell::set_fullscreen(&surface, false);
    }

    fn new_popup(&mut self, surface: PopupSurface, positioner: PositionerState) {
        crate::shell::new_popup(self, surface, positioner);
    }

    fn reposition_request(&mut self, surface: PopupSurface, positioner: PositionerState, token: u32) {
        crate::shell::reposition(self, &surface, positioner);
        surface.send_repositioned(token);
    }

    fn grab(&mut self, _surface: PopupSurface, _seat: wl_seat::WlSeat, _serial: Serial) {}

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        crate::shell::toplevel_destroyed(self, &surface);
    }

    fn popup_destroyed(&mut self, _surface: PopupSurface) {
        self.schedule_frame();
    }

    fn title_changed(&mut self, surface: ToplevelSurface) {
        crate::shell::update_identity(self, &surface);
    }

    fn app_id_changed(&mut self, surface: ToplevelSurface) {
        crate::shell::update_identity(self, &surface);
    }
}

impl SeatHandler for State {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<State> {
        &mut self.seat_state
    }

    fn cursor_image(&mut self, _seat: &Seat<Self>, image: CursorImageStatus) {
        self.cursor = image;
        crate::stream::cursor_changed(self);
    }

    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&WlSurface>) {
        let client = focused.and_then(|s| self.dh.get_client(s.id()).ok());
        smithay::wayland::selection::data_device::set_data_device_focus(&self.dh, seat, client);
    }
}

impl SelectionHandler for State {
    /// The text of a selection the compositor owns (a person's paste).
    type SelectionUserData = std::sync::Arc<str>;

    fn new_selection(
        &mut self,
        ty: smithay::wayland::selection::SelectionTarget,
        source: Option<smithay::wayland::selection::SelectionSource>,
        _seat: Seat<Self>,
    ) {
        if ty == smithay::wayland::selection::SelectionTarget::Clipboard {
            crate::copy::selection_changed(self, source);
        }
    }

    /// Writes the pasted text on its own thread, so a client that reads slowly gets all of it and one that never reads
    /// cannot hold the event loop: each write waits at most [`PASTE_DEADLINE`] in all, and at most [`PASTE_WRITERS`]
    /// run at once (a further request gets nothing).
    fn send_selection(
        &mut self,
        _ty: smithay::wayland::selection::SelectionTarget,
        _mime_type: String,
        fd: std::os::fd::OwnedFd,
        _seat: Seat<Self>,
        text: &std::sync::Arc<str>,
    ) {
        write_later(fd, text.clone());
    }
}

/// Writes `text` to `fd` on its own thread, within [`PASTE_DEADLINE`], at most [`PASTE_WRITERS`] at once (beyond that the
/// reader gets nothing): a paste, or the files a drag offers.
pub fn write_later(fd: std::os::fd::OwnedFd, text: std::sync::Arc<str>) {
    write_later_then(fd, text, || {});
}

/// As [`write_later`], then runs `written` once the whole text is written and the descriptor closed (never when it was
/// given up on).
pub fn write_later_then(fd: std::os::fd::OwnedFd, text: std::sync::Arc<str>, written: impl FnOnce() + Send + 'static) {
    use std::sync::atomic::Ordering;
    if PASTE_WRITERS_RUNNING.fetch_add(1, Ordering::Relaxed) >= PASTE_WRITERS {
        PASTE_WRITERS_RUNNING.fetch_sub(1, Ordering::Relaxed);
        return;
    }
    // Held (and counted) while the thread writes it, mostly past the request that handed it over.
    let fd = crate::relay::Held::new(fd);
    let spawned = std::thread::Builder::new().name("paste".into()).spawn(move || {
        let complete = write_within(&fd, text.as_bytes(), PASTE_DEADLINE);
        drop(fd);
        PASTE_WRITERS_RUNNING.fetch_sub(1, Ordering::Relaxed);
        if complete {
            written();
        }
    });
    if spawned.is_err() {
        PASTE_WRITERS_RUNNING.fetch_sub(1, Ordering::Relaxed);
    }
}

/// How long a paste may take to write in all.
const PASTE_DEADLINE: std::time::Duration = std::time::Duration::from_secs(2);
/// Pastes written at once.
const PASTE_WRITERS: usize = 4;
static PASTE_WRITERS_RUNNING: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Writes all of `bytes` to `fd` within `deadline`, waiting in `poll` between partial writes; whether it did.
fn write_within(fd: &impl std::os::fd::AsFd, mut bytes: &[u8], deadline: std::time::Duration) -> bool {
    use rustix::event::{PollFd, PollFlags, Timespec};
    use rustix::fs::{OFlags, fcntl_getfl, fcntl_setfl};
    let Ok(flags) = fcntl_getfl(fd) else { return false };
    if fcntl_setfl(fd, flags | OFlags::NONBLOCK).is_err() {
        return false;
    }
    let end = std::time::Instant::now() + deadline;
    while !bytes.is_empty() {
        match rustix::io::write(fd, bytes) {
            Ok(0) => return false,
            Ok(n) => bytes = &bytes[n..],
            Err(rustix::io::Errno::INTR) => {}
            Err(rustix::io::Errno::AGAIN) => {
                let left = end.saturating_duration_since(std::time::Instant::now());
                let Ok(timeout) = Timespec::try_from(left) else { return false };
                if left.is_zero() {
                    return false;
                }
                let mut fds = [PollFd::new(fd, PollFlags::OUT)];
                if rustix::event::poll(&mut fds, Some(&timeout)).is_err() {
                    return false;
                }
            }
            Err(_) => return false,
        }
    }
    true
}

impl DataDeviceHandler for State {
    fn data_device_state(&mut self) -> &mut DataDeviceState {
        &mut self.data_device
    }
}

// Required by the cursor-shape global; there is no tablet.
impl smithay::input::tablet::TabletSeatHandler for State {
    type ToolFocus = WlSurface;
}

/// A drag's icon surface and its offset from the pointer, moved by the client's `wl_surface.offset`.
pub struct DragIcon {
    pub surface: WlSurface,
    pub offset: smithay::utils::Point<i32, smithay::utils::Logical>,
}

impl smithay::input::dnd::DndGrabHandler for State {
    fn dropped(
        &mut self,
        _target: Option<smithay::input::dnd::DndTarget<'_, Self>>,
        _validated: bool,
        _seat: Seat<Self>,
        _location: smithay::utils::Point<f64, smithay::utils::Logical>,
    ) {
        self.drag_icon = None;
        self.schedule_frame();
    }

    fn cancelled(&mut self, _seat: Seat<Self>, _location: smithay::utils::Point<f64, smithay::utils::Logical>) {
        self.drag_icon = None;
        self.schedule_frame();
    }
}

/// A drag the browser starts from a pointer press on its own surface (an image or a link dragged within the page, or
/// out of it): the pointer then carries it, its icon drawn under the pointer, until the release drops it. Only the
/// browser may start one, only during the press it asks for (a click grab, never the compositor's own drags), and only
/// from and with its own surfaces.
impl WaylandDndGrabHandler for State {
    fn dnd_requested<S: smithay::input::dnd::Source>(
        &mut self,
        source: S,
        icon: Option<WlSurface>,
        seat: Seat<Self>,
        serial: Serial,
        type_: smithay::input::dnd::GrabType,
    ) {
        let browser = |client: Option<Client>| client.as_ref().and_then(client_kind) == Some(ClientKind::Browser);
        let source_any: &dyn std::any::Any = &source;
        let requester = source_any
            .downcast_ref::<wayland_server::protocol::wl_data_source::WlDataSource>()
            .and_then(Resource::client)
            .or_else(|| source_any.downcast_ref::<WlSurface>().and_then(Resource::client));
        let start = seat.get_pointer().and_then(|pointer| Some((pointer.grab_start_data()?, pointer)));
        let (smithay::input::dnd::GrabType::Pointer, Some((start, pointer))) = (type_, start) else {
            source.cancel();
            return;
        };
        let clicked = pointer.with_grab(|current, grab| {
            current == serial && grab.as_any().is::<smithay::input::pointer::ClickGrab<State>>()
        });
        let from_browser = start.focus.as_ref().is_some_and(|(surface, _)| browser(surface.client()));
        if clicked != Some(true)
            || !browser(requester)
            || !from_browser
            || icon.as_ref().is_some_and(|icon| !browser(icon.client()))
        {
            source.cancel();
            return;
        }
        self.drag_icon = icon.map(|surface| DragIcon { surface, offset: (0, 0).into() });
        let grab = smithay::input::dnd::DnDGrab::new_pointer(&self.dh, start, source, seat);
        pointer.set_grab(self, grab, serial, smithay::input::pointer::Focus::Keep);
        self.schedule_frame();
    }
}

impl OutputHandler for State {}

impl smithay::wayland::pointer_constraints::PointerConstraintsHandler for State {}

impl ForeignToplevelListHandler for State {
    fn foreign_toplevel_list_state(&mut self) -> &mut ForeignToplevelListState {
        &mut self.foreign_toplevels
    }
}

impl VirtualPointerHandler for State {
    fn process_virtual_pointer_event(&mut self, event: InputEvent<VirtualPointerBackend>) {
        crate::input::agent_pointer_event(self, event);
    }
}

// What `smithay::delegate_dispatch2!(State)` writes, except that two requests are checked before smithay sees them,
// since smithay acts on them before any handler of the compositor's runs. An `xdg_positioner` request: smithay computes
// a popup's geometry from its positioner in plain `i32` as it creates the popup, so the bound must hold when the
// positioner takes its values. A `zwlr_virtual_pointer_v1.button`: smithay tracks every code it is sent as held, after
// a scan of those it holds, so codes outside the mouse buttons would grow that list and its scans without bound.
impl<I, UserData> wayland_server::Dispatch<I, UserData> for State
where
    I: Resource,
    I::Request: 'static,
    UserData: smithay::wayland::Dispatch2<I, State>,
{
    fn request(
        state: &mut Self,
        client: &Client,
        resource: &I,
        request: I::Request,
        data: &UserData,
        dh: &DisplayHandle,
        data_init: &mut wayland_server::DataInit<'_, Self>,
    ) {
        use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_positioner;
        use smithay::reexports::wayland_protocols_wlr::virtual_pointer::v1::server::zwlr_virtual_pointer_v1;
        let any = &request as &dyn std::any::Any;
        let positioner = any.downcast_ref::<xdg_positioner::Request>();
        if positioner.is_some_and(|request| !crate::shell::positioner_in_bounds(request)) {
            resource.post_error(xdg_positioner::Error::InvalidInput, "positioner geometry out of bounds");
            return;
        }
        if let Some(zwlr_virtual_pointer_v1::Request::Button { button, .. }) =
            any.downcast_ref::<zwlr_virtual_pointer_v1::Request>()
            && !crate::input::is_mouse_button(*button)
        {
            // The protocol has no error for it; its first code is as good as any to name the object.
            resource.post_error(0u32, "a button code outside the mouse buttons");
            return;
        }
        data.request(state, client, resource, request, dh, data_init);
    }

    fn destroyed(state: &mut Self, client: ClientId, resource: &I, data: &UserData) {
        data.destroyed(state, client, resource);
    }
}

impl<I, UserData> wayland_server::GlobalDispatch<I, UserData> for State
where
    I: Resource,
    UserData: smithay::wayland::GlobalDispatch2<I, State>,
{
    fn bind(
        state: &mut Self,
        dh: &DisplayHandle,
        client: &Client,
        resource: wayland_server::New<I>,
        data: &UserData,
        data_init: &mut wayland_server::DataInit<'_, Self>,
    ) {
        data.bind(state, dh, client, resource, data_init);
    }

    fn can_view(client: Client, data: &UserData) -> bool {
        data.can_view(&client)
    }
}

pub fn popup_kind(surface: &PopupSurface) -> PopupKind {
    PopupKind::Xdg(surface.clone())
}

pub fn toplevel_identity(surface: &ToplevelSurface) -> (String, String) {
    with_states(surface.wl_surface(), |states| {
        let data = states.data_map.get::<XdgToplevelSurfaceData>().expect("toplevel data").lock().unwrap();
        (data.title.clone().unwrap_or_default(), data.app_id.clone().unwrap_or_default())
    })
}
