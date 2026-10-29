//! easi.run controller: the only Docker-socket holder, exposing a closed set of session operations.
//!
//! Configuration comes from the environment:
//! - `CONTROLLER_INSTANCE` — this installation's name, default `easirun`: every session resource's name starts with it
//!   and carries it as a label, and cleanup touches only its own, so two installations can share one Docker Engine.
//! - `CONTROLLER_SOCKET` — the Unix socket it serves, default `/run/controller/controller.sock` (mode 0660, the
//!   group of its directory).
//! - `CONTROLLER_ADMIN_UID` — the only peer uid it accepts, default the admin's (10004). A test hook: the displays
//!   still accept only the admin's uid.
//! - `IMAGE_DISPLAY`, `IMAGE_BROWSER`, `IMAGE_AGENT`, `IMAGE_PROXY` — the session images, each pinned by
//!   `@sha256:`, or a plain local tag (`name:tag`) that is resolved once at start to its immutable image id
//!   (`sha256:...`), which every template then uses.
//! - `CONTROLLER_BROWSER_SECCOMP` — the browser seccomp profile (JSON), default
//!   `/etc/controller/browser-seccomp.json`.
//! - `CONTROLLER_BROWSER_CPUS` and `CONTROLLER_BROWSER_MEMORY_MIB` — each browser container's limits, default 2
//!   CPUs and 2048 MiB (see `template::BrowserLimits`).
//! - `STREAMS_VOLUME` — the long-lived volume holding each session's stream socket directory.
//! - `CONTROLLER_MAX_SESSIONS` — the most sessions it holds at once, 1-16, default 16 (the admin's own
//!   `ADMIN_MAX_SESSIONS` may be at most 16 too): it refuses any creation past it, whatever the admin asks. A session
//!   whose rollback was incomplete, or whose containers an earlier run left, keeps counting until it is stopped or
//!   cleaned up, since its resources still exist.
//! - The `bench` profile only, all three or none: `CONTROLLER_BENCH_NETWORK` (the fixture host's network, which session
//!   proxies join), `CONTROLLER_BENCH_FIXTURE` (`name=ipv4`, the one destination they admit beyond production policy),
//!   and `CONTROLLER_BENCH_SPKI_FILE` (the fixture certificate's SPKI hash: a key Chromium accepts despite its
//!   certificate's errors, besides what it trusts anyway). With them, optionally `CONTROLLER_BENCH_LANES`:
//!   space-separated `BROWSER/DISPLAY/MEMS` cpuset lists, the CPU lanes a session may be placed in (`bench/runner/lanes.py` writes them), and
//!   `CONTROLLER_BENCH_FIXTURE_ONLY=1`, the public demo's walled garden: proxies send every destination to the fixture
//!   and the session networks are internal. It refuses to start unless the fixture network is internal too, and needs
//!   `CONTROLLER_MAX_SESSION_SECONDS`: any session older is stopped, whatever the admin does. That variable is refused
//!   anywhere else.
//!
//! It refuses to start on a Docker Engine older than 28, the oldest the project is tested on.

mod guard;
mod image;
mod ops;
mod request;
mod server;
mod template;

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use bollard::Docker;
use display_protocol::session::MAX_SESSIONS;

use crate::image::Images;
use crate::ops::Controller;
use crate::template::Fixed;

const DOCKER_SOCKET: &str = "/var/run/docker.sock";
const MIN_ENGINE_MAJOR: u32 = 28;
const MAX_SECCOMP: u64 = 1024 * 1024;

struct Config {
    instance: String,
    /// The walled garden's session age limit (`CONTROLLER_MAX_SESSION_SECONDS`), required there and refused elsewhere.
    max_session: Option<std::time::Duration>,
    max_sessions: usize,
    socket: PathBuf,
    admin_uid: u32,
    /// `(variable, reference)` in display, browser, agent, proxy order; tags are resolved after connecting.
    images: [(&'static str, String); 4],
    browser: template::BrowserLimits,
    browser_seccomp: String,
    streams_volume: String,
    bench: Option<template::Bench>,
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

fn required(name: &str) -> Result<String, String> {
    env(name).ok_or(format!("{name} is required"))
}

/// `CONTROLLER_INSTANCE`: this installation's name, default `easirun`. Lowercase letters, digits and dashes, starting
/// with a letter, at most 16 bytes: it starts every resource name and labels every resource, and every cleanup is
/// limited to it.
fn instance() -> Result<String, String> {
    let instance = env("CONTROLLER_INSTANCE").unwrap_or_else(|| "easirun".into());
    let ok = instance.len() <= 16
        && instance.bytes().next().is_some_and(|b| b.is_ascii_lowercase())
        && instance.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if ok { Ok(instance) } else { Err(format!("bad CONTROLLER_INSTANCE {instance:?}")) }
}

/// `CONTROLLER_MAX_SESSIONS`: 1 to `MAX_SESSIONS`, default `MAX_SESSIONS`.
fn max_sessions(text: Option<&str>) -> Result<usize, String> {
    match text {
        None => Ok(MAX_SESSIONS),
        Some(text) => text
            .parse()
            .ok()
            .filter(|n| (1..=MAX_SESSIONS).contains(n))
            .ok_or(format!("CONTROLLER_MAX_SESSIONS must be 1-{MAX_SESSIONS}")),
    }
}

fn config() -> Result<Config, String> {
    let instance = instance()?;
    let socket = PathBuf::from(env("CONTROLLER_SOCKET").unwrap_or_else(|| "/run/controller/controller.sock".into()));
    let admin_uid = match env("CONTROLLER_ADMIN_UID") {
        Some(text) => text.parse().map_err(|_| "bad CONTROLLER_ADMIN_UID")?,
        None => template::UID_ADMIN,
    };
    let image = |name| required(name).map(|reference| (name, reference));
    let images = [image("IMAGE_DISPLAY")?, image("IMAGE_BROWSER")?, image("IMAGE_AGENT")?, image("IMAGE_PROXY")?];
    let seccomp_path =
        env("CONTROLLER_BROWSER_SECCOMP").unwrap_or_else(|| "/etc/controller/browser-seccomp.json".into());
    let size = std::fs::metadata(&seccomp_path).map_err(|e| format!("{seccomp_path}: {e}"))?.len();
    if size > MAX_SECCOMP {
        return Err(format!("{seccomp_path} is larger than {MAX_SECCOMP} bytes"));
    }
    let text = std::fs::read_to_string(&seccomp_path).map_err(|e| format!("{seccomp_path}: {e}"))?;
    let profile: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("{seccomp_path}: {e}"))?;
    if !profile.get("defaultAction").is_some_and(serde_json::Value::is_string) {
        return Err(format!("{seccomp_path} is not a seccomp profile (no defaultAction)"));
    }
    let streams_volume = required("STREAMS_VOLUME")?;
    let name_ok = streams_volume.bytes().next().is_some_and(|b| b.is_ascii_alphanumeric())
        && streams_volume.bytes().all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b));
    if !name_ok || streams_volume.starts_with(&format!("{instance}-")) {
        return Err(format!("STREAMS_VOLUME must be a plain volume name outside the {instance}- session namespace"));
    }
    // Compact: the profile travels inline as `seccomp=<json>`.
    let browser_seccomp = profile.to_string();
    let browser = template::BrowserLimits::parse(
        env("CONTROLLER_BROWSER_CPUS").as_deref(),
        env("CONTROLLER_BROWSER_MEMORY_MIB").as_deref(),
    )?;
    let bench = bench(&instance)?;
    let max_sessions = max_sessions(env("CONTROLLER_MAX_SESSIONS").as_deref())?;
    let max_session = match env("CONTROLLER_MAX_SESSION_SECONDS") {
        None => None,
        Some(text) => Some(std::time::Duration::from_secs(
            text.parse()
                .ok()
                .filter(|s| (10..=3600).contains(s))
                .ok_or("CONTROLLER_MAX_SESSION_SECONDS must be 10-3600")?,
        )),
    };
    // Only the walled garden owns every session outright: elsewhere the admin keeps a place the reaper would empty,
    // and the start-up cleanup would end every session under way.
    if bench.as_ref().is_some_and(|b| b.fixture_only) != max_session.is_some() {
        return Err("CONTROLLER_MAX_SESSION_SECONDS goes with CONTROLLER_BENCH_FIXTURE_ONLY=1, and only there".into());
    }
    Ok(Config {
        instance,
        socket,
        admin_uid,
        images,
        browser,
        browser_seccomp,
        streams_volume,
        bench,
        max_session,
        max_sessions,
    })
}

/// The bench profile's fixture exception: all of `CONTROLLER_BENCH_NETWORK`, `CONTROLLER_BENCH_FIXTURE`
/// (`name=ipv4`), and `CONTROLLER_BENCH_SPKI_FILE`, or none of them.
fn bench(instance: &str) -> Result<Option<template::Bench>, String> {
    let vars = (env("CONTROLLER_BENCH_NETWORK"), env("CONTROLLER_BENCH_FIXTURE"), env("CONTROLLER_BENCH_SPKI_FILE"));
    let (network, fixture, spki_file) = match vars {
        (None, None, None) if env("CONTROLLER_BENCH_LANES").is_some() => {
            return Err("CONTROLLER_BENCH_LANES needs the bench profile".into());
        }
        (None, None, None) if env("CONTROLLER_BENCH_FIXTURE_ONLY").is_some() => {
            return Err("CONTROLLER_BENCH_FIXTURE_ONLY needs the bench profile".into());
        }
        (None, None, None) => return Ok(None),
        (Some(n), Some(f), Some(p)) => (n, f, p),
        _ => return Err("the CONTROLLER_BENCH_* variables go together".into()),
    };
    // Refused as every session proxy would refuse it at its own start.
    egress::destination::BenchFixture::parse(&fixture).map_err(|e| format!("CONTROLLER_BENCH_FIXTURE: {e}"))?;
    if network.starts_with(&format!("{instance}-")) {
        return Err(format!("CONTROLLER_BENCH_NETWORK must be outside {instance}-"));
    }
    let lanes = env("CONTROLLER_BENCH_LANES")
        .map(|text| text.split_whitespace().map(template::Lane::parse).collect::<Result<Vec<_>, _>>())
        .transpose()?
        .unwrap_or_default();
    if lanes.len() > usize::from(u8::MAX) {
        return Err("CONTROLLER_BENCH_LANES declares too many lanes".into());
    }
    let fixture_only = match env("CONTROLLER_BENCH_FIXTURE_ONLY").as_deref() {
        None => false,
        Some("1") => true,
        Some(_) => return Err("CONTROLLER_BENCH_FIXTURE_ONLY: expected 1".into()),
    };
    tracing::warn!(%fixture, %network, lanes = lanes.len(), fixture_only, "bench profile: session proxies admit the fixture host");
    Ok(Some(template::Bench { network, fixture, spki_file: spki_file.into(), lanes, fixture_only }))
}

/// A local tag becomes its image id, once; a digest-pinned reference stays as it is; `Images::new` refuses the rest.
async fn resolve(docker: &Docker, name: &str, reference: &str) -> Result<String, String> {
    if !image::local_tag(reference) {
        return Ok(reference.to_owned());
    }
    let id = docker.inspect_image(reference).await.map_err(|e| format!("{name}: {reference}: {e}"))?.id;
    let id = id.ok_or(format!("{name}: {reference} has no image id"))?;
    tracing::info!(variable = name, tag = reference, %id, "resolved a local tag to its image id");
    Ok(id)
}

async fn images(docker: &Docker, mut images: [(&'static str, String); 4]) -> Result<Images, String> {
    for (name, reference) in &mut images {
        *reference = resolve(docker, name, reference).await?;
    }
    Images::new(images)
}

async fn connect(path: &str) -> Result<Docker, String> {
    let docker = Docker::connect_with_unix(path, 120, bollard::API_DEFAULT_VERSION).map_err(|e| e.to_string())?;
    let docker = docker.negotiate_version().await.map_err(|e| format!("docker: {e}"))?;
    let version = docker.version().await.map_err(|e| format!("docker: {e}"))?.version.unwrap_or_default();
    let major: u32 = version.split('.').next().and_then(|m| m.parse().ok()).unwrap_or(0);
    if major < MIN_ENGINE_MAJOR {
        return Err(format!("Docker Engine {version:?} is older than {MIN_ENGINE_MAJOR}"));
    }
    tracing::info!(%version, "Docker Engine");
    Ok(docker)
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err((code, why)) => {
            eprintln!("controller: {why}");
            code
        }
    }
}

/// Serves until terminated; an error is the exit code, 2 for a configuration it refuses, and its message.
async fn run() -> Result<(), (ExitCode, String)> {
    let failure = |why: String| (ExitCode::FAILURE, why);
    let refused = |why: String| (ExitCode::from(2), why);
    // Non-dumpable before anything else: the kernel makes no core dump of it, whatever the host's core pattern (a
    // collector the pattern pipes crashes to gets them at any core limit but 1), and no other process of its uid may
    // read its memory.
    rustix::process::set_dumpable_behavior(rustix::process::DumpableBehavior::NotDumpable)
        .map_err(|why| failure(format!("cannot make itself non-dumpable: {why}")))?;
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let config = config().map_err(refused)?;
    let docker = connect(DOCKER_SOCKET).await.map_err(failure)?;
    if let Some(bench) = config.bench.as_ref().filter(|b| b.fixture_only) {
        // The session networks are internal; a fixture network with a route out would undo that.
        match docker.inspect_network(&bench.network, None).await {
            Ok(network) if network.internal == Some(true) => {}
            Ok(_) => {
                return Err(refused(format!("the walled garden's fixture network {} must be internal", bench.network)));
            }
            Err(e) => return Err(refused(format!("the fixture network {}: {e}", bench.network))),
        }
    }
    let fixed = Fixed {
        instance: config.instance,
        images: images(&docker, config.images).await.map_err(refused)?,
        browser: config.browser,
        browser_seccomp: config.browser_seccomp,
        streams_volume: config.streams_volume,
        bench: config.bench,
    };
    let listener = server::bind(&config.socket).map_err(failure)?;
    tracing::info!(socket = %config.socket.display(), admin_uid = config.admin_uid, "listening");
    let controller = Arc::new(Controller::new(docker, fixed, config.max_sessions));
    // A restart (the controller alone, while the admin keeps running) must not double the session limit.
    let left =
        controller.inherit().await.map_err(|why| failure(format!("could not list what an earlier run left: {why}")))?;
    if left > 0 {
        tracing::warn!(left, "sessions an earlier run left count toward the limit until stopped or cleaned up")
    }
    if let Some(max) = config.max_session {
        // A restarted walled garden holds no records of what an earlier run left: it removes it before serving, so
        // nothing escapes the age limit.
        controller
            .cleanup()
            .await
            .map_err(|why| failure(format!("the walled garden could not remove what an earlier run left: {why}")))?;
        let reaper = controller.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(5));
            loop {
                tick.tick().await;
                reaper.reap(max).await;
            }
        });
    }
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(|e| failure(e.to_string()))?;
    tokio::select! {
        () = server::serve(listener, controller, config.admin_uid) => {}
        _ = terminate.recv() => tracing::info!("terminated"),
        _ = tokio::signal::ctrl_c() => tracing::info!("interrupted"),
    }
    let _ = std::fs::remove_file(&config.socket);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_session_limit_defaults_to_the_admins_maximum_and_refuses_anything_outside_it() {
        assert_eq!(max_sessions(None), Ok(16));
        assert_eq!(max_sessions(Some("1")), Ok(1));
        assert_eq!(max_sessions(Some("10")), Ok(10));
        assert_eq!(max_sessions(Some("16")), Ok(16));
        for bad in ["0", "17", "-1", "4.5", " 4", "four", "18446744073709551617"] {
            assert!(max_sessions(Some(bad)).is_err(), "{bad:?} was accepted");
        }
    }
}
