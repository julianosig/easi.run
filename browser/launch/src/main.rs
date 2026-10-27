//! The browser container's entrypoint, as uid 10001: the proxy forwarder, the session bus, the accessibility bus, the
//! sound daemon, and Chrome in an ordinary maximized window on the display's Wayland socket. It runs no shell. Chrome
//! closing on its own (its last window closed) opens it again; the launcher exits as soon as Chromium fails, or one of
//! the buses or the sound daemon stops.
//!
//! Environment: `BROWSER_URL`, `BROWSER_PROXY_SOCKET` (the proxy's Unix socket, which the loopback forwarder reaches),
//! `BROWSER_SIZE` (`WIDTHxHEIGHT`), `BROWSER_TIMEZONE` (an IANA name), `BROWSER_LANGUAGES` (BCP 47 tags, preferred
//! first, comma-separated), `BROWSER_DEVTOOLS` (`1` opens DevTools on loopback), `BROWSER_PROFILE` (`1`: the session
//! starts from a saved profile, which Chrome waits for and which is saved at the session's end, see `profile`), and in
//! the bench profile `BROWSER_BENCH_SPKI` and `BROWSER_KIOSK` (`1`: kiosk mode, the page alone on the screen as on the
//! benchmark's baseline).
//!
//! `browser-launch profile-restore` and `browser-launch profile-save` are the profile's two commands, which the
//! controller runs beside the launcher (see `profile`).

mod cdp;
mod downloads;
mod files;
mod forward;
mod gate;
mod netns;
mod portal;
mod profile;

/// A fresh directory for one test, removed when dropped.
#[cfg(test)]
struct Scratch(std::path::PathBuf);

#[cfg(test)]
impl Scratch {
    fn new(name: &str) -> Scratch {
        static MADE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let made = MADE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("launch-{name}-{}-{made}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    /// The directory, as the launcher's paths are written.
    fn path(&self) -> &str {
        self.0.to_str().unwrap()
    }

    /// A new directory `name` in it.
    fn dir(&self, name: &str) -> std::path::PathBuf {
        let dir = self.0.join(name);
        std::fs::create_dir(&dir).unwrap();
        dir
    }
}

#[cfg(test)]
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::Path;
use std::process::{Child, Command, ExitCode};
use std::time::{Duration, Instant};

const RUNTIME: &str = "/run/browser/bus";
const WAYLAND: &str = "/run/browser/wayland/wayland-0";
const SESSION_BUS: &str = "unix:path=/run/browser/bus/session";
const BUS_GID: u32 = 10012;
const CHROMIUM: &str = "/opt/google/chrome/chrome";
/// How many times Chromium is opened again after closing on its own; each relaunch keeps its DevTools pipe's two
/// descriptors open (Chromium closes itself when its pipe goes away), so the count is bounded.
const MAX_RELAUNCHES: u32 = 50;
/// The pause before it opens again.
const RELAUNCH_PAUSE: Duration = Duration::from_millis(300);
/// The display's FIFO, which the sound daemon writes Chromium's mixed sound to as 48 kHz stereo 16-bit samples.
const AUDIO_FIFO: &str = "/run/browser/wayland/audio";
/// How long the launcher waits for what the display and the proxy open (the Wayland socket, the sound FIFO, the proxy
/// socket). The controller starts their containers beside this one, and Docker under load (sessions starting and
/// stopping at once) may take more than 10 s to start them. A display that never comes up fails the session's start
/// on the admin's own bound.
const PEERS_WITHIN: Duration = Duration::from_secs(60);
/// The sound daemon's private directory and socket, outside the bus volume the agent shares.
const PULSE_DIR: &str = "/home/browser/pulse";
const PULSE_SOCKET: &str = "/home/browser/pulse/native";

fn required(name: &str) -> Result<String, String> {
    std::env::var(name).ok().filter(|v| !v.is_empty()).ok_or(format!("{name} is required"))
}

fn command(program: &str) -> Command {
    let mut command = Command::new(program);
    command
        .env("HOME", "/home/browser")
        .env("XDG_RUNTIME_DIR", RUNTIME)
        // UTF-8 file names: without it Chromium cannot name a download "relatório.pdf" and saves it as "download".
        .env("LANG", "C.UTF-8")
        .env("DBUS_SESSION_BUS_ADDRESS", SESSION_BUS)
        .env("WAYLAND_DISPLAY", WAYLAND)
        .env("ACCESSIBILITY_ENABLED", "1")
        .env("NO_AT_BRIDGE", "0");
    command
}

fn wait_for(paths: &[&str], deadline: Duration) -> Result<(), String> {
    let end = Instant::now() + deadline;
    while !paths.iter().all(|p| Path::new(p).exists()) {
        if Instant::now() > end {
            let missing: Vec<&&str> = paths.iter().filter(|p| !Path::new(p).exists()).collect();
            return Err(format!("{missing:?} did not appear"));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}

/// The agent reaches the accessibility bus through the bus group.
fn share_accessibility_bus() -> Result<(), String> {
    let dir = format!("{RUNTIME}/at-spi");
    let socket = format!("{dir}/bus");
    for (path, mode) in [(&dir, 0o750), (&socket, 0o660)] {
        std::os::unix::fs::chown(path, None, Some(BUS_GID)).map_err(|e| format!("{path}: {e}"))?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).map_err(|e| format!("{path}: {e}"))?;
    }
    Ok(())
}

/// PulseAudio as Chromium's own uid, so it holds nothing Chromium lacks: no configuration files, no module loading by
/// clients, no shared memory, no real-time scheduling, and only three modules, the native protocol on a private socket,
/// the pipe sink to the display's FIFO (paced by the system clock, so the display only reads), and suspend on idle,
/// which stops the samples when nothing plays.
fn pulseaudio() -> Result<Child, String> {
    std::fs::create_dir_all(PULSE_DIR).map_err(|e| format!("{PULSE_DIR}: {e}"))?;
    std::fs::set_permissions(PULSE_DIR, std::fs::Permissions::from_mode(0o700))
        .map_err(|e| format!("{PULSE_DIR}: {e}"))?;
    command("/usr/bin/pulseaudio")
        .env("XDG_RUNTIME_DIR", PULSE_DIR)
        .env("PULSE_RUNTIME_PATH", PULSE_DIR)
        .env("PULSE_STATE_PATH", PULSE_DIR)
        .env_remove("DBUS_SESSION_BUS_ADDRESS")
        .args(["-n", "--daemonize=no", "--system=no", "--use-pid-file=no", "--exit-idle-time=-1", "--disallow-exit"])
        .args(["--disallow-module-loading", "--realtime=no", "--high-priority=no", "--disable-shm=yes"])
        .args(["--log-target=stderr", "--log-level=error"])
        .args(["-L", &format!("module-native-protocol-unix socket={PULSE_SOCKET} auth-anonymous=1")])
        .args(["-L", &format!("module-pipe-sink file={AUDIO_FIFO} sink_name=display format=s16le rate=48000 channels=2 use_system_clock_for_timing=yes")])
        .args(["-L", "module-suspend-on-idle timeout=1"])
        .spawn()
        .map_err(|e| format!("pulseaudio: {e}"))
}

/// Chromium's proxy: the browser has no network, and this loopback forwarder carries each connection unchanged to the
/// proxy's Unix socket.
const FORWARDER: &str = "127.0.0.1:3128";
/// Where Chromium saves downloads (a tmpfs private to the browser's container).
const DOWNLOADS: &str = "/downloads";
/// Chromium's temporary directory, on the download directory's file system: a page it saves is written here, then moved
/// into the download directory, which is then a rename that `downloads` delivers (from /tmp, another file system, it
/// would be a copy written in place, which is never taken). Made before the downloads are watched, so it is not one.
const TEMPORARY: &str = "/downloads/.tmp";

/// Creates `path` as a new file of mode 0600, refusing anything already there, a symbolic link included.
fn create_new(path: impl rustix::path::Arg) -> rustix::io::Result<std::os::fd::OwnedFd> {
    let flags = rustix::fs::OFlags::WRONLY
        | rustix::fs::OFlags::CREATE
        | rustix::fs::OFlags::EXCL
        | rustix::fs::OFlags::NOFOLLOW
        | rustix::fs::OFlags::CLOEXEC;
    rustix::fs::open(path, flags, rustix::fs::Mode::from_raw_mode(0o600))
}

fn chromium_arguments((width, height): (u16, u16), devtools: bool) -> Result<Vec<String>, String> {
    let mut args: Vec<String> = [
        "--ozone-platform=wayland",
        // An ordinary maximized window, with its tab strip and omnibox: a kiosk window fills the screen with the page,
        // which bot detectors read as headless.
        "--start-maximized",
        "--no-first-run",
        "--no-default-browser-check",
        // WebGL on Mesa's llvmpipe (software, as on any Linux machine without a GPU driver): Chrome blocks software
        // renderers unless told otherwise, and without WebGL a browser reads as a bot. Composition stays in software,
        // into the shared-memory buffers the compositor reads.
        "--ignore-gpu-blocklist",
        "--disable-gpu-compositing",
        "--force-renderer-accessibility",
        "--user-data-dir=/home/browser/profile",
        // Cookies and saved site data encrypted with Chrome's fixed key rather than a keyring's, which the container
        // has none of: a saved profile then reads its cookies in the next session. The saved profile's own encryption
        // is what protects them at rest (docs/security.md).
        "--password-store=basic",
        "--disk-cache-dir=/tmp/cache",
        // The launcher's own DevTools pipe (descriptors 3 and 4): download sizes, the tabs it opens and closes, and the
        // browser's close as a persistent profile is saved (see cdp.rs). The pipe marks the browser as automated
        // (navigator.webdriver), which it is not: a person or the agent drives it through input, as without the pipe,
        // and sites' bot checks would otherwise refuse the person.
        "--remote-debugging-pipe",
        "--disable-blink-features=AutomationControlled",
        "--proxy-bypass-list=<-loopback>",
        "--disable-background-networking",
        "--disable-component-update",
        // No spare renderer kept warm for the next site: one page at a time does not need it, and it holds 7-9 MiB.
        // ParallelDownloading would write a download out of order; the launcher streams it in order.
        "--disable-features=MediaRouter,Translate,SpareRendererForSitePerProcess,ParallelDownloading",
        // The profile's tmpfs reads as an ordinary disk (so pages see the usual storage quota); this bounds what one
        // site may store to 192 MiB (three quarters of the pool), and the pool is evicted beyond 256 MiB. The
        // browser's memory limit bounds the rest (docs/security.md).
        "--enable-features=StorageQuotaSettings:PoolSizeBytes/268435456",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    args.push(format!("--proxy-server=http://{FORWARDER}"));
    // The window starts at the screen's size, so the first page never loads in a smaller one and then grows.
    args.extend([format!("--window-size={width},{height}"), "--window-position=0,0".to_owned()]);
    if let Ok(spki) = required("BROWSER_BENCH_SPKI") {
        // The bench profile only: the fixture host's key is accepted despite its certificate's errors.
        args.push(format!("--ignore-certificate-errors-spki-list={spki}"));
    }
    if devtools {
        args.extend(["--remote-debugging-port=9222".into(), "--remote-debugging-address=127.0.0.1".into()]);
    }
    if std::env::var("BROWSER_KIOSK").is_ok_and(|v| v == "1") {
        args.push("--kiosk".into());
    }
    args.push("--".into());
    args.push(required("BROWSER_URL")?);
    Ok(args)
}

/// Holds descriptors 3 and 4 from the start (on /dev/null, closed on exec), before any thread or child exists, so
/// nothing else takes them and only Chromium inherits its DevTools pipe there.
fn reserve_pipe_fds() -> Result<[OwnedFd; 2], String> {
    let open = || null().map_err(|e| format!("/dev/null: {e}"));
    let reserved = [open()?, open()?];
    if reserved[0].as_raw_fd() != 3 || reserved[1].as_raw_fd() != 4 {
        return Err("descriptors 3 and 4 were not free for Chromium's DevTools pipe".into());
    }
    Ok(reserved)
}

fn null() -> rustix::io::Result<OwnedFd> {
    rustix::fs::open("/dev/null", rustix::fs::OFlags::RDWR | rustix::fs::OFlags::CLOEXEC, rustix::fs::Mode::empty())
}

/// Spawns Chromium with its DevTools pipe on descriptors 3 (commands) and 4 (messages): the pipe ends are placed there
/// just for the spawn, then 3 and 4 go back to /dev/null, closed on exec, whatever happened. The launcher keeps the
/// other ends.
fn spawn_with_pipe(command: &mut Command, reserved: &mut [OwnedFd; 2]) -> Result<(Child, OwnedFd, OwnedFd), String> {
    use rustix::pipe::{PipeFlags, pipe_with};
    let pipe = || pipe_with(PipeFlags::CLOEXEC).map_err(|e| format!("DevTools pipe: {e}"));
    let ((commands_in, commands), (messages, messages_out)) = (pipe()?, pipe()?);
    let placed = rustix::io::dup2(&commands_in, &mut reserved[0])
        .and_then(|()| rustix::io::dup2(&messages_out, &mut reserved[1]));
    let spawned = placed
        .map_err(|e| format!("DevTools pipe: {e}"))
        .and_then(|()| command.spawn().map_err(|e| format!("chromium: {e}")));
    let restored = null().and_then(|null| {
        for fd in reserved.iter_mut() {
            rustix::io::dup2(&null, fd)?;
            rustix::io::fcntl_setfd(&*fd, rustix::io::FdFlags::CLOEXEC)?;
        }
        Ok(())
    });
    restored.map_err(|e| format!("descriptors 3 and 4: {e}"))?;
    Ok((spawned?, commands, messages))
}

/// The longest preferences file a restored profile's settings are kept from.
const MAX_PREFERENCES: u64 = 16 * 1024 * 1024;

/// The profile Chrome starts with (its tmpfs is new each session, empty or restored from a saved profile): its window
/// placement says maximized at the screen's size, so the first page lays out in the window it keeps rather than in a
/// smaller one that then grows. Its accepted languages are the session's, so navigator.languages and Accept-Language
/// list them in order. A restored profile keeps every other preference Chrome saved.
fn seed_profile(width: u16, height: u16, languages: &[String]) -> Result<(), String> {
    let dir = format!("{}/Default", profile::PROFILE);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{dir}: {e}"))?;
    let file = format!("{dir}/Preferences");
    let mut preferences = saved_preferences(&file).unwrap_or_else(|| serde_json::json!({}));
    let browser = member(&mut preferences, "browser");
    *member(browser, "custom_chrome_frame") = serde_json::json!(false);
    *member(browser, "window_placement") = serde_json::json!({
        "maximized": true, "left": 0, "top": 0, "right": width, "bottom": height,
        "work_area_left": 0, "work_area_top": 0, "work_area_right": width, "work_area_bottom": height,
    });
    let intl = member(&mut preferences, "intl");
    *member(intl, "accept_languages") = serde_json::json!(languages.join(","));
    *member(intl, "selected_languages") = serde_json::json!(languages.join(","));
    std::fs::write(&file, preferences.to_string()).map_err(|e| format!("{file}: {e}"))
}

/// The preferences a restored profile holds, if they are a JSON object of a bounded size.
fn saved_preferences(file: &str) -> Option<serde_json::Value> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(file).ok()?.take(MAX_PREFERENCES + 1).read_to_end(&mut bytes).ok()?;
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).ok().filter(|_| bytes.len() as u64 <= MAX_PREFERENCES)?;
    value.is_object().then_some(value)
}

/// `value`'s member `key`, `value` made an object first if it is not one.
fn member<'a>(value: &'a mut serde_json::Value, key: &str) -> &'a mut serde_json::Value {
    if !value.is_object() {
        *value = serde_json::json!({});
    }
    value.as_object_mut().expect("an object").entry(key).or_insert(serde_json::Value::Null)
}

/// The session's time zone, checked again here (the controller already did).
fn timezone() -> Result<String, String> {
    let name = required("BROWSER_TIMEZONE")?;
    if !locale::valid_timezone(&name) {
        return Err(format!("BROWSER_TIMEZONE is not an IANA time zone: {name:?}"));
    }
    Ok(name)
}

/// The session's languages, checked again here.
fn languages() -> Result<Vec<String>, String> {
    let list = required("BROWSER_LANGUAGES")?;
    let tags: Vec<String> = list.split(',').map(str::to_owned).collect();
    if !locale::valid_languages(&tags) {
        return Err(format!("BROWSER_LANGUAGES is not a list of language tags: {list:?}"));
    }
    Ok(tags)
}

fn screen_size() -> Result<(u16, u16), String> {
    let size = required("BROWSER_SIZE")?;
    size.split_once('x')
        .and_then(|(w, h)| Some((w.parse::<u16>().ok()?, h.parse::<u16>().ok()?)))
        .ok_or_else(|| format!("BROWSER_SIZE is not WIDTHxHEIGHT: {size}"))
}

fn run() -> Result<ExitCode, String> {
    let mut reserved = reserve_pipe_fds()?;
    let (width, height) = screen_size()?;
    let languages = languages()?;
    // A saved profile appears whole, once the admin restored it (see `profile`).
    let saved = std::env::var("BROWSER_PROFILE").is_ok_and(|v| v == "1");
    if saved {
        wait_for(&[profile::PROFILE], profile::RESTORE_WAIT)?;
    }
    seed_profile(width, height, &languages)?;
    let timezone = timezone()?;
    let socket = required("BROWSER_PROXY_SOCKET")?;
    let devtools = std::env::var("BROWSER_DEVTOOLS").is_ok_and(|v| v == "1");
    let mut children: Vec<(&str, Child)> = Vec::new();
    // The buses first, in the container's own user namespace: they check their peers' uids, and the agent's
    // (another uid) would not be mapped in the launcher's.
    let bus = command("/usr/bin/dbus-daemon")
        .args(["--config-file=/etc/display-browser/session.conf", "--nofork", "--nopidfile"])
        .spawn()
        .map_err(|e| format!("dbus-daemon: {e}"))?;
    children.push(("the session bus", bus));
    wait_for(&["/run/browser/bus/session"], Duration::from_secs(10))?;
    let a11y = command("/usr/libexec/at-spi-bus-launcher")
        .arg("--launch-immediately")
        .spawn()
        .map_err(|e| format!("at-spi-bus-launcher: {e}"))?;
    children.push(("the accessibility bus", a11y));
    let a11y_socket = format!("{RUNTIME}/at-spi/bus");
    wait_for(&[&a11y_socket], Duration::from_secs(10))?;
    wait_for(&[WAYLAND, AUDIO_FIFO, &socket], PEERS_WITHIN)?;
    share_accessibility_bus()?;
    // Then Chrome's own network namespace with an online link (see netns), before any thread starts. Not with DevTools:
    // the agent's driver would have to join it, and every process of this uid outside it holds its namespace's
    // capabilities, so the driver would rest on the buses' integrity; there Chrome reads offline, as without a link.
    if !devtools {
        netns::isolate()?;
    }
    // Non-dumpable from here, before Chrome starts and pages send it anything: the kernel makes no core dump of the
    // launcher, whatever the host's core pattern (a collector the pattern pipes crashes to gets them at any core limit
    // but 1), and no other process of its uid may read its memory. Not before: a non-dumpable process may not write its
    // own uid_map, which netns::isolate does. Chrome and the buses exec afresh and are not covered.
    profile::non_dumpable()?;
    let gate = gate::Gate::new(DOWNLOADS);
    forward::start(FORWARDER, &socket, gate.clone()).map_err(|e| format!("proxy forwarder: {e}"))?;
    children.push(("the sound daemon", pulseaudio()?));
    wait_for(&[PULSE_SOCKET], Duration::from_secs(10))?;
    // Before Chromium starts, so the files global and the file chooser portal are the launcher's alone.
    let link = files::bind(WAYLAND)?;
    let shared = portal::Shared::new(link.files.clone(), link.conn.clone());
    portal::serve(shared.clone(), SESSION_BUS)?;
    let sizes = std::sync::Arc::new(cdp::Sizes::default());
    // Pages the display asks for are answered on the same Wayland connection.
    let pages = {
        let (files, conn) = (link.files.clone(), link.conn.clone());
        cdp::Pages::new(move |request, opened| {
            files.page_opened(request, u32::from(opened));
            let _ = conn.flush();
        })
    };
    std::fs::DirBuilder::new().mode(0o700).create(TEMPORARY).map_err(|e| format!("{TEMPORARY}: {e}"))?;
    files::watch(link, DOWNLOADS, shared.clone(), gate, sizes.clone(), pages.clone());
    let enabled = command("/usr/bin/dbus-send")
        .args(["--session", "--dest=org.a11y.Bus", "/org/a11y/bus", "org.freedesktop.DBus.Properties.Set"])
        .args(["string:org.a11y.Status", "string:IsEnabled", "variant:boolean:true"])
        .status()
        .map_err(|e| format!("dbus-send: {e}"))?;
    if !enabled.success() {
        return Err("could not enable accessibility on the session bus".into());
    }
    let mut command = command(CHROMIUM);
    command
        .env("PULSE_SERVER", format!("unix:{PULSE_SOCKET}"))
        .env("PULSE_CLIENTCONFIG", "/etc/display-browser/pulse-client.conf")
        // Pages read their time zone from this (Chrome carries its own zone data), and on Linux Chrome takes its
        // interface and pages' default locale (Intl) from LANGUAGE; LANG stays C.UTF-8 (no other locale is installed).
        .env("TMPDIR", TEMPORARY)
        .env("TZ", timezone)
        .env("LANGUAGE", locale::gettext_list(&languages))
        .args(chromium_arguments((width, height), devtools)?);
    let mut launch = || -> Result<Child, String> {
        // Forgotten before the new Chromium can start a download, so none of the earlier one's sizes is matched to it.
        let generation = sizes.renew();
        let (chromium, commands, messages) = spawn_with_pipe(&mut command, &mut reserved)?;
        cdp::start(sizes.clone(), pages.clone(), generation, commands, messages);
        shared.started(chromium.id());
        Ok(chromium)
    };
    let mut chromium = launch()?;
    let mut relaunches = 0;
    let mut closing = profile::Closing::default();
    // Exit as soon as any of them stops, except Chromium closing on its own (its last window closed, or the person quit
    // it): it opens again at the session's first page, with the same profile. Closed for a saved profile's save, it
    // stays closed while the profile is packed, until the session ends. init reaps the rest when the container ends.
    loop {
        for (name, child) in &mut children {
            if let Ok(Some(status)) = child.try_wait() {
                eprintln!("browser-launch: {name} exited ({status})");
                return Ok(ExitCode::from(status.code().unwrap_or(1).clamp(1, 255) as u8));
            }
        }
        if saved && closing.turn(&mut chromium, || pages.close_browser()) {
            std::thread::sleep(Duration::from_millis(200));
            continue;
        }
        if let Ok(Some(status)) = chromium.try_wait() {
            if !status.success() || relaunches >= MAX_RELAUNCHES {
                eprintln!("browser-launch: chromium exited ({status}) after {relaunches} relaunches");
                return Ok(ExitCode::from(status.code().unwrap_or(1).clamp(1, 255) as u8));
            }
            relaunches += 1;
            eprintln!("browser-launch: chromium closed; opening it again");
            std::thread::sleep(RELAUNCH_PAUSE);
            chromium = launch()?;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        None => {}
        Some("profile-restore") => return profile::restore(),
        Some("profile-save") => return profile::save(),
        Some(other) => {
            eprintln!(
                "browser-launch: unknown command {other:?}; usage: browser-launch [profile-restore | profile-save]"
            );
            return ExitCode::from(2);
        }
    }
    // Chromium, and every process it starts, inherits this.
    affinity::stay_on_cores();
    match run() {
        Ok(code) => code,
        Err(why) => {
            eprintln!("browser-launch: {why}");
            ExitCode::FAILURE
        }
    }
}
