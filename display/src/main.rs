//! easi.run: a headless Wayland compositor for one browser.
//!
//! Configuration comes from the environment:
//! - `DISPLAY_OUTPUT` — `WIDTHxHEIGHT`, as `OUTPUT_SIZE_RULE` (protocol/src/frame.rs) says: even, from 320x240 to
//!   1920x1080, and not a size SVT-AV1 crashes on; default `1280x800`.
//! - `DISPLAY_MAX_FPS` — the output's advertised refresh rate, by which clients pace their drawing, default 60.
//! - `DISPLAY_COMPOSE_GAP_MS` — the shortest time between two compositions, default 4: a commit is composed as soon as
//!   it arrives, at most this often.
//! - `DISPLAY_BROWSER_SOCKET` and `DISPLAY_BROWSER_UID` — the browser's Wayland socket and the only uid it accepts.
//! - `DISPLAY_AGENT_SOCKET`, `DISPLAY_INJECT_SOCKET`, and `DISPLAY_AGENT_UID` — the agent's privileged Wayland socket,
//!   its `cua-inject v1` socket, and the only uid they accept. Both sockets are optional together.
//! - `DISPLAY_STREAM_SOCKET` and `DISPLAY_ADMIN_UID` — the viewer stream socket and the only uid it accepts.
//! - `DISPLAY_AUDIO_FIFO` — optional, with the stream socket: the FIFO the browser's sound arrives on, which the
//!   display creates in the browser socket's directory, shared with its group.
//! - `DISPLAY_BACKGROUND` — the color shown where no window is, around the logo, as `#rrggbb`; default the brand's
//!   wash.
//! - `DISPLAY_LOGO` — the logo shown on it in place of easi.run's: the base64 of a PNG `display_protocol::logo` takes,
//!   which the controller reads from its `CONTROLLER_DISPLAY_LOGO_FILE`.

mod background;
mod compose;
mod copy;
mod drop;
mod files;
mod frame;
mod inject;
mod input;
mod limits;
mod recording;
mod relay;
mod screencopy;
mod shell;
mod shm;
mod state;
mod stream;
mod uploads;
mod virtual_keyboard;
mod wlr_toplevels;

use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::Instant;

use calloop::generic::Generic;
use calloop::timer::{TimeoutAction, Timer};
use calloop::{EventLoop, Interest, Mode, PostAction};
use smithay::desktop::PopupManager;
use smithay::input::SeatState;
use smithay::input::keyboard::XkbConfig;
use smithay::output::{Mode as OutputMode, Output, PhysicalProperties, Subpixel};
use smithay::reexports::wayland_protocols_misc::zwp_virtual_keyboard_v1::server::zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1;
use smithay::wayland::compositor::CompositorState;
use smithay::wayland::foreign_toplevel_list::ForeignToplevelListState;
use smithay::wayland::output::OutputManagerState;
use smithay::wayland::selection::data_device::DataDeviceState;
use smithay::wayland::shell::xdg::XdgShellState;
use smithay::wayland::virtual_pointer::VirtualPointerManagerState;
use wayland_protocols_wlr::foreign_toplevel::v1::server::zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1;
use wayland_protocols_wlr::screencopy::v1::server::zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1;
use wayland_server::protocol::wl_shm::WlShm;
use wayland_server::{Display, ListeningSocket, Resource};

use crate::files::protocol::easi_files_v1::EasiFilesV1;
use crate::state::{ClientKind, Data, State, client_kind};

struct Config {
    width: u32,
    height: u32,
    max_fps: u32,
    compose_gap_ms: u64,
    browser_socket: PathBuf,
    browser_uid: u32,
    agent: Option<(PathBuf, PathBuf, u32)>,
    stream: Option<(PathBuf, u32)>,
    audio_fifo: Option<PathBuf>,
    background: u32,
    logo: display_protocol::logo::Logo,
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// The output's size from `DISPLAY_OUTPUT`, within what the protocol's video carries
/// ([`display_protocol::frame::output_size_ok`]).
fn output(text: &str) -> Result<(u32, u32), String> {
    display_protocol::frame::parse_output_size(text)
        .ok_or_else(|| format!("DISPLAY_OUTPUT must be WIDTHxHEIGHT, {}", display_protocol::frame::OUTPUT_SIZE_RULE))
}

/// `DISPLAY_BACKGROUND`, as the environment holds it: a `#rrggbb` color, or the brand's wash when it is unset or
/// empty. Anything else, a value that is not UTF-8 included, is refused rather than taken as unset.
fn background(value: Result<String, std::env::VarError>) -> Result<u32, String> {
    match value {
        Err(std::env::VarError::NotPresent) => Ok(background::WASH),
        Ok(text) if text.is_empty() => Ok(background::WASH),
        Ok(text) => display_protocol::frame::parse_color(&text)
            .ok_or_else(|| format!("DISPLAY_BACKGROUND must be a #rrggbb color, not {text:?}")),
        Err(std::env::VarError::NotUnicode(_)) => {
            Err("DISPLAY_BACKGROUND must be a #rrggbb color, not a value that is not UTF-8".into())
        }
    }
}

/// `DISPLAY_LOGO`, as the environment holds it: the logo it carries, or easi.run's when it is unset or empty. One
/// [`display_protocol::logo::decode_text`] refuses, or that is not UTF-8, is refused.
fn logo(value: Result<String, std::env::VarError>) -> Result<display_protocol::logo::Logo, String> {
    match value {
        Err(std::env::VarError::NotPresent) => Ok(background::easi_run()),
        Ok(text) if text.is_empty() => Ok(background::easi_run()),
        Ok(text) => display_protocol::logo::decode_text(&text).map_err(|why| format!("DISPLAY_LOGO: {why}")),
        Err(std::env::VarError::NotUnicode(_)) => Err("DISPLAY_LOGO: not base64 (not even UTF-8)".into()),
    }
}

fn config() -> Result<Config, String> {
    let (width, height) = output(&env("DISPLAY_OUTPUT").unwrap_or_else(|| "1280x800".into()))?;
    let max_fps =
        env("DISPLAY_MAX_FPS").map(|v| v.parse().map_err(|_| "bad DISPLAY_MAX_FPS")).transpose()?.unwrap_or(60);
    if !(1..=120).contains(&max_fps) {
        return Err("DISPLAY_MAX_FPS must be 1-120".into());
    }
    let compose_gap_ms = env("DISPLAY_COMPOSE_GAP_MS")
        .map(|v| v.parse().map_err(|_| "bad DISPLAY_COMPOSE_GAP_MS"))
        .transpose()?
        .unwrap_or(4);
    if !(1..=1000).contains(&compose_gap_ms) {
        return Err("DISPLAY_COMPOSE_GAP_MS must be 1-1000".into());
    }
    let uid = |name: &str| -> Result<u32, String> {
        env(name).ok_or(format!("{name} is required"))?.parse().map_err(|_| format!("bad {name}"))
    };
    let browser_socket = PathBuf::from(env("DISPLAY_BROWSER_SOCKET").ok_or("DISPLAY_BROWSER_SOCKET is required")?);
    let browser_uid = uid("DISPLAY_BROWSER_UID")?;
    let agent = match (env("DISPLAY_AGENT_SOCKET"), env("DISPLAY_INJECT_SOCKET")) {
        (Some(socket), Some(inject)) => Some((socket.into(), inject.into(), uid("DISPLAY_AGENT_UID")?)),
        (None, None) => None,
        _ => return Err("DISPLAY_AGENT_SOCKET and DISPLAY_INJECT_SOCKET go together".into()),
    };
    let stream = match env("DISPLAY_STREAM_SOCKET") {
        Some(path) => Some((path.into(), uid("DISPLAY_ADMIN_UID")?)),
        None => None,
    };
    let audio_fifo = env("DISPLAY_AUDIO_FIFO").map(PathBuf::from);
    if audio_fifo.is_some() && stream.is_none() {
        return Err("DISPLAY_AUDIO_FIFO needs DISPLAY_STREAM_SOCKET".into());
    }
    let background = background(std::env::var("DISPLAY_BACKGROUND"))?;
    let logo = logo(std::env::var("DISPLAY_LOGO"))?;
    Ok(Config {
        width,
        height,
        max_fps,
        compose_gap_ms,
        browser_socket,
        browser_uid,
        agent,
        stream,
        audio_fifo,
        background,
        logo,
    })
}

fn main() {
    // Dumpable, unlike easi.run's other programs: the hostile-client tests prove its descriptor bounds by reading its
    // /proc/<pid>/fd as its uid, which a non-dumpable process refuses. It runs alone in its container's process
    // namespace, and its container's core limit of 1 leaves no core dump.
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let config = match config() {
        Ok(config) => config,
        Err(why) => {
            eprintln!("display: {why}");
            std::process::exit(2);
        }
    };
    if let Err(why) = run(config) {
        eprintln!("display: {why}");
        std::process::exit(1);
    }
}

/// The kernel-attested uid of a Unix-socket peer. The peer's pid is 0 when it lives in another PID namespace (another
/// container), which this tolerates; only the uid is used.
pub fn peer_uid(stream: &impl std::os::fd::AsFd) -> Option<u32> {
    nix::sys::socket::getsockopt(stream, nix::sys::socket::sockopt::PeerCredentials).ok().map(|c| c.uid())
}

/// Serves `listener`'s connections from `uid` alone, each to `on_accept`; one from another uid is closed and logged as
/// `label` connection ("an injection", "a stream").
pub fn listen_unix(
    handle: &calloop::LoopHandle<'static, Data>,
    listener: UnixListener,
    uid: u32,
    label: &'static str,
    mut on_accept: impl FnMut(UnixStream, &mut Data) + 'static,
) {
    listener.set_nonblocking(true).expect("non-blocking listener");
    handle
        .insert_source(Generic::new(listener, Interest::READ, Mode::Level), move |_, listener, data| {
            while let Ok((stream, _)) = listener.accept() {
                let peer = peer_uid(&stream);
                if peer != Some(uid) {
                    tracing::warn!(?peer, "refused {label} connection from another uid");
                    continue;
                }
                on_accept(stream, data);
            }
            Ok(PostAction::Continue)
        })
        .expect("listener source");
}

/// Gives a socket its directory's group and mode 0660, so exactly the directory's group can connect.
fn share_with_directory_group(path: &std::path::Path) -> Result<(), String> {
    let parent = path.parent().ok_or("a socket path needs a directory")?;
    let gid = rustix::fs::stat(parent).map_err(|e| format!("{}: {e}", parent.display()))?.st_gid;
    rustix::fs::chown(path, None, Some(rustix::fs::Gid::from_raw(gid)))
        .map_err(|e| format!("{}: {e}", path.display()))?;
    rustix::fs::chmod(path, rustix::fs::Mode::from_raw_mode(0o660)).map_err(|e| format!("{}: {e}", path.display()))
}

/// A Unix socket listening at `path`, in place of what was there, shared with its directory's group.
fn bind_shared_unix(path: &std::path::Path) -> Result<UnixListener, String> {
    let _ = std::fs::remove_file(path);
    let listener = UnixListener::bind(path).map_err(|e| format!("{}: {e}", path.display()))?;
    share_with_directory_group(path)?;
    Ok(listener)
}

fn listen_wayland(
    handle: &calloop::LoopHandle<'static, Data>,
    path: PathBuf,
    kind: ClientKind,
    uid: u32,
) -> Result<(), String> {
    let socket = ListeningSocket::bind_absolute(path.clone()).map_err(|e| format!("{}: {e}", path.display()))?;
    share_with_directory_group(&path)?;
    handle
        .insert_source(Generic::new(socket, Interest::READ, Mode::Level), move |_, socket, data| {
            while let Ok(Some(stream)) = socket.accept() {
                let peer = peer_uid(&stream);
                if peer != Some(uid) {
                    tracing::warn!(?kind, ?peer, "refused a Wayland connection from another uid");
                    continue;
                }
                if let Err(e) = relay::insert(&mut data.state, stream, kind) {
                    tracing::warn!(%e, "could not insert a client");
                }
            }
            Ok(PostAction::Continue)
        })
        .map_err(|e| e.to_string())?;
    tracing::info!(path = %path.display(), ?kind, uid, "listening");
    Ok(())
}

/// Sets the soft descriptor limit to [`limits::MAX_OPEN_FDS`].
fn limit_descriptors() -> Result<(), String> {
    use rustix::process::{Resource, Rlimit, getrlimit, setrlimit};
    let wanted = limits::MAX_OPEN_FDS as u64;
    let hard = getrlimit(Resource::Nofile).maximum;
    if let Some(hard) = hard
        && hard < wanted
    {
        return Err(format!("the hard descriptor limit {hard} is below the {wanted} the compositor needs"));
    }
    setrlimit(Resource::Nofile, Rlimit { current: Some(wanted), maximum: hard })
        .map_err(|e| format!("the descriptor limit: {e}"))
}

fn run(config: Config) -> Result<(), String> {
    limit_descriptors()?;
    if let Some((cpu, cores)) = affinity::stay_on_cores() {
        tracing::info!(cpu, %cores, "threads kept on the quota's physical cores");
    }
    let mut event_loop: EventLoop<'static, Data> = EventLoop::try_new().map_err(|e| e.to_string())?;
    let mut display: Display<State> = Display::new().map_err(|e| e.to_string())?;
    let dh = display.handle();

    let agent_only = |client: &wayland_server::Client| client_kind(client) == Some(ClientKind::Agent);
    let compositor = CompositorState::new::<State>(&dh);
    let xdg_shell = XdgShellState::new::<State>(&dh);
    // Server-side decorations (the compositor draws none): Chrome then reserves no shadow around a window that is not
    // maximized yet, so its first page already lays out at the width it keeps.
    smithay::wayland::shell::xdg::decoration::XdgDecorationState::new::<State>(&dh);
    // The globals these states create live in the display; the states hold nothing the compositor reads again.
    OutputManagerState::new_with_xdg_output::<State>(&dh);
    let data_device = DataDeviceState::new::<State>(&dh);
    // Presentation times on the monotonic clock: Chromium times its frames by them (see frame.rs).
    smithay::wayland::presentation::PresentationState::new::<State>(&dh, rustix::time::ClockId::Monotonic as u32);
    // Chromium names its cursors through this, so viewers get shapes instead of images.
    smithay::wayland::cursor_shape::CursorShapeManagerState::new::<State>(&dh);
    let foreign_toplevels = ForeignToplevelListState::new_with_filter::<State>(&dh, agent_only);
    // The agent's virtual devices: their globals, too, live in the display.
    VirtualPointerManagerState::new::<State, _>(&dh, agent_only);
    dh.create_global::<State, WlShm, _>(1, shm::ShmGlobal);
    dh.create_global::<State, ZwlrScreencopyManagerV1, _>(screencopy::VERSION, screencopy::ScreencopyGlobal);
    dh.create_global::<State, ZwlrForeignToplevelManagerV1, _>(wlr_toplevels::VERSION, wlr_toplevels::ManagerGlobal);
    dh.create_global::<State, ZwpVirtualKeyboardManagerV1, _>(
        virtual_keyboard::VERSION,
        virtual_keyboard::ManagerGlobal,
    );
    dh.create_global::<State, EasiFilesV1, _>(EasiFilesV1::interface().version, files::FilesGlobal);

    let mut seat_state = SeatState::new();
    let mut seat = seat_state.new_wl_seat(&dh, "seat0");
    // Repeat rate 0: Chrome repeats no key itself. A viewer sends each repeat of a held key as a press of its own, so
    // the admin's record of what the person typed sees every character a held Backspace deletes.
    seat.add_keyboard(XkbConfig::default(), 600, 0).map_err(|e| e.to_string())?;
    seat.add_pointer();

    let output = Output::new(
        "HEADLESS-1".into(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "easi.run".into(),
            model: "Display".into(),
            serial_number: String::new(),
        },
    );
    let mode = OutputMode {
        size: (config.width as i32, config.height as i32).into(),
        refresh: (config.max_fps * 1000) as i32,
    };
    output.change_current_state(Some(mode), None, None, Some((0, 0).into()));
    output.set_preferred(mode);
    let _output_global = output.create_global::<State>(&dh);

    let handle = event_loop.handle();
    // Requests reach the library through the relays, which dispatch them (see `relay`); this source still reaps the
    // clients that closed.
    let poll_fd = display.backend().poll_fd().try_clone_to_owned().map_err(|e| e.to_string())?;
    handle
        .insert_source(Generic::new(poll_fd, Interest::READ, Mode::Level), |_, _, data: &mut Data| {
            data.display.dispatch_clients(&mut data.state).map_err(std::io::Error::other)?;
            let _ = data.display.flush_clients();
            Ok(PostAction::Continue)
        })
        .map_err(|e| e.to_string())?;

    // A still screen keeps no spare frame buffers: every few seconds without a composition, they are released.
    handle
        .insert_source(Timer::from_duration(frame::IDLE), |_, _, data: &mut Data| {
            frame::release_when_idle(&mut data.state);
            TimeoutAction::ToDuration(frame::IDLE)
        })
        .map_err(|e| e.to_string())?;

    // Before the browser's socket: the browser waits for both.
    let audio_fifo = config.audio_fifo.as_deref().map(stream::audio::create).transpose()?;
    listen_wayland(&handle, config.browser_socket.clone(), ClientKind::Browser, config.browser_uid)?;
    if let Some((socket, inject_path, uid)) = &config.agent {
        listen_wayland(&handle, socket.clone(), ClientKind::Agent, *uid)?;
        inject::listen(&handle, bind_shared_unix(inject_path)?, *uid);
        tracing::info!(path = %inject_path.display(), "injection socket listening");
    }

    if let Some((path, uid)) = &config.stream {
        stream::listen(&handle, bind_shared_unix(path)?, *uid, config.agent.is_some());
        tracing::info!(path = %path.display(), uid, "stream socket listening");
    }

    let state = State {
        start: Instant::now(),
        dh,
        handle: handle.clone(),
        compositor,
        xdg_shell,
        seat_state,
        data_device,
        foreign_toplevels,
        popups: PopupManager::default(),
        popup_grab: None,
        seat,
        output,
        toplevels: Vec::new(),
        toplevel_states_stale: false,
        clients: Vec::new(),
        wlr_managers: Vec::new(),
        scene: compose::Scene::new(
            config.width as i32,
            config.height as i32,
            background::draw(config.width as i32, config.height as i32, config.background, &config.logo),
        ),
        cursor: smithay::input::pointer::CursorImageStatus::default_named(),
        drag_icon: None,
        buttons: Vec::new(),
        agent_keyboards: Default::default(),
        lease: input::Lease::default(),
        typing: input::Typing::default(),
        copy: copy::Job::default(),
        files: files::Files::default(),
        uploads: uploads::Uploads::default(),
        inject_binding: None,
        pending_text: None,
        chars: inject::CharTable::new(),
        hub: stream::Hub::default(),
        recordings: recording::Recordings::default(),
        copies: screencopy::Copies::default(),
        compose_gap: std::time::Duration::from_millis(config.compose_gap_ms),
        last_frame: Instant::now(),
        presentation_seq: 0,
    };
    if let Some(fifo) = audio_fifo {
        stream::audio::start(fifo, state.hub.viewers.clone(), state.recordings.tap.clone());
        tracing::info!("the browser's sound goes to viewers");
    }
    let mut data = Data { state, display };
    event_loop
        .run(None, &mut data, |data| {
            data.state.popups.cleanup();
            let _ = data.display.flush_clients();
        })
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_output_follows_the_protocol_s_size_rule() {
        assert_eq!(output("1280x800"), Ok((1280, 800)));
        for refused in ["1922x1080", "1281x800", "1280", "x800", "1280x"] {
            assert!(output(refused).is_err(), "{refused}");
        }
    }

    #[test]
    fn the_background_is_a_color_or_the_wash_and_nothing_else() {
        use std::env::VarError;
        use std::os::unix::ffi::OsStringExt;
        assert_eq!(background(Err(VarError::NotPresent)), Ok(background::WASH));
        assert_eq!(background(Ok(String::new())), Ok(background::WASH));
        assert_eq!(background(Ok("#102030".into())), Ok(0x102030));
        assert!(background(Ok("#10203".into())).is_err());
        let not_utf8 = std::ffi::OsString::from_vec(vec![b'#', 0xff, b'0', b'0', b'0', b'0', b'0']);
        assert!(background(Err(VarError::NotUnicode(not_utf8))).is_err(), "not taken as unset");
    }

    #[test]
    fn the_logo_is_a_png_display_protocol_takes_or_easi_run_s() {
        use std::env::VarError;
        use std::os::unix::ffi::OsStringExt;
        assert_eq!(logo(Err(VarError::NotPresent)), Ok(background::easi_run()));
        assert_eq!(logo(Ok(String::new())), Ok(background::easi_run()));
        let mut png = Vec::new();
        let mut encoder = png::Encoder::new(&mut png, 1, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.write_header().unwrap().write_image_data(&[1, 2, 3, 4]).unwrap();
        let decoded = logo(Ok(display_protocol::logo::encode(&png))).unwrap();
        assert_eq!((decoded.width, decoded.height, decoded.rgba), (1, 1, vec![1, 2, 3, 4]));
        assert!(logo(Ok("iVBORw0KGgo=".into())).unwrap_err().starts_with("DISPLAY_LOGO: "), "a PNG cut short");
        assert!(logo(Err(VarError::NotUnicode(std::ffi::OsString::from_vec(vec![0xff])))).is_err());
    }

    #[test]
    fn a_logo_s_transparent_color_shows_the_background() {
        // Red beside blue, red the transparent color (tRNS).
        let mut png = Vec::new();
        let mut encoder = png::Encoder::new(&mut png, 2, 1);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_trns(vec![0, 0xff, 0, 0, 0, 0]);
        encoder.write_header().unwrap().write_image_data(&[0xff, 0, 0, 0, 0, 0xff]).unwrap();
        let logo = logo(Ok(display_protocol::logo::encode(&png))).unwrap();
        let (width, height) = (1280, 800);
        let pixels = background::draw(width, height, 0x123456, &logo);
        // Drawn at its own size, two pixels from (639, 399).
        let at = |x: usize| pixels[399 * width as usize + x];
        assert_eq!((at(638), at(639), at(640), at(641)), (0xff12_3456, 0xff12_3456, 0xff00_00ff, 0xff12_3456));
    }
}
