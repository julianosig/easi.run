//! The controller's only exec in an agent container: the Cua Driver's MCP server on its standard streams.
//!
//! This program and the driver are both execute-only, so the kernel marks each non-dumpable from its exec: no other
//! process of the same uid (Chromium, in DevTools mode) can read their memory, descriptors, or mount namespace through
//! /proc, not even in the moment before this program execs the driver.

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
    // The driver's sockets, its Wayland backend, and its home come in the container's environment, which the
    // controller's template writes next to the mounts they name.
    command.env("XDG_RUNTIME_DIR", runtime).env("CUA_DRIVER_RS_TELEMETRY_ENABLED", "false").env("DO_NOT_TRACK", "1");
    let error = command.exec();
    eprintln!("agent-mcp: {DRIVER}: {error}");
    std::process::exit(127);
}
