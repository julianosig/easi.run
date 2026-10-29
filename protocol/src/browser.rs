//! Where the browser container finds what it shares with the display and the agent: the directories and group the
//! controller gives it (`controller/src/template.rs`), and the sockets the browser's launcher finds in them.

/// The display's Wayland socket and sound FIFO for the browser, read-only: the display's own directory for them,
/// mounted here.
pub const WAYLAND_DIR: &str = "/run/browser/wayland";
/// The buses' runtime directory, which the agent mounts at the same path, so the addresses the buses publish resolve
/// in both.
pub const BUS_DIR: &str = "/run/browser/bus";
/// The group the agent reaches the buses through.
pub const BUS_GID: u32 = 10012;
/// The display's Wayland socket's and sound FIFO's names, in its directory and so in [`WAYLAND_DIR`].
pub const WAYLAND_SOCKET: &str = "wayland-0";
pub const AUDIO_FIFO: &str = "audio";

/// The Wayland socket the browser draws on.
pub fn wayland_socket() -> String {
    format!("{WAYLAND_DIR}/{WAYLAND_SOCKET}")
}

/// The display's FIFO, which the browser's sound daemon writes Chromium's mixed sound to as 48 kHz stereo 16-bit
/// samples.
pub fn audio_fifo() -> String {
    format!("{WAYLAND_DIR}/{AUDIO_FIFO}")
}

/// The session bus's socket, and its address (`DBUS_SESSION_BUS_ADDRESS`) in the browser and the agent.
pub fn session_bus() -> String {
    format!("{BUS_DIR}/session")
}

pub fn session_bus_address() -> String {
    format!("unix:path={}", session_bus())
}
