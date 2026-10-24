//! The controller's only exec in an agent container: the Cua Driver's MCP server on its standard streams.
//!
//! The driver binary is execute-only, so executing it from here makes the kernel mark the driver non-dumpable: no
//! other process of the same uid (Chromium, in DevTools mode) can read its memory, descriptors, or mount namespace
//! through /proc. This program is readable on purpose; it is the driver that must not be.

use std::os::unix::fs::DirBuilderExt;
use std::os::unix::process::CommandExt;
use std::process::Command;

const DRIVER: &str = "/usr/local/bin/cua-driver";

fn main() {
    let runtime = "/tmp/runtime";
    let _ = std::fs::DirBuilder::new().recursive(true).mode(0o700).create(runtime);
    let mut command = Command::new(DRIVER);
    command.arg("mcp");
    if std::env::var("AGENT_DEVTOOLS").is_ok_and(|v| v == "1") {
        command.args(["--grant", "existing-profile"]);
    }
    command
        .env("HOME", "/tmp")
        .env("XDG_RUNTIME_DIR", runtime)
        .env("CUA_DRIVER_RS_ENABLE_WAYLAND", "1")
        .env("CUA_DRIVER_RS_TELEMETRY_ENABLED", "false")
        .env("DO_NOT_TRACK", "1")
        .env("WAYLAND_DISPLAY", "/run/agent/wayland-0")
        .env("CUA_INJECT_SOCKET", "/run/agent/cua-inject")
        .env("DBUS_SESSION_BUS_ADDRESS", "unix:path=/run/browser/bus/session");
    let error = command.exec();
    eprintln!("agent-mcp: {DRIVER}: {error}");
    std::process::exit(127);
}
