//! easi.run egress proxy: the browser container's only route to the internet.
//!
//! It accepts `CONNECT host:port` (ports 80 and 443) and absolute-form plain HTTP on port 80, resolves each name
//! itself exactly once, refuses the request when any answer is a non-public address, and dials a vetted address.
//!
//! Configuration comes from the environment:
//! - `PROXY_LISTEN` — the listen address, default `0.0.0.0:3128`.
//! - `PROXY_LISTEN_UNIX` and `PROXY_CLIENT_UID` — instead, a Unix socket that only that uid may use: the browser has
//!   no network, and the browser launcher's loopback forwarder brings Chromium's proxy connections here.
//! - `PROXY_MAX_TUNNELS` — concurrent requests, default 256; excess connections get 503.
//! - `PROXY_BENCH_FIXTURE` — `name=ipv4`, set only by the `bench` profile: that exact name maps to that exact
//!   address without DNS. Absent (or empty) in production, which has no exception.
//! - `PROXY_UPSTREAM_STDIN` — `1` when the session's traffic leaves through the deployment's upstream proxy: it arrives
//!   as one JSON line on standard input (never in the environment, which `docker inspect` shows). Without a valid one
//!   within five seconds the proxy exits; with one, every connection and every name resolution goes through it, and
//!   once listening the proxy prints `{"ready":true}` as its first line on standard output, which the controller
//!   awaits.

mod doh;
mod log;
mod policy;
mod request;
mod resolve;
mod server;
mod upstream;

#[cfg(test)]
mod tests;

use std::net::SocketAddr;
use std::process::ExitCode;
use std::sync::Arc;

use tokio::net::TcpListener;
use tokio::signal::unix::{SignalKind, signal};

use egress::destination::BenchFixture;
use tokio::io::AsyncReadExt;

use crate::resolve::{Resolver, SystemResolver};
use crate::server::Config;
use crate::upstream::Dialer;

/// The largest upstream line read from standard input, and how long it may take to arrive.
const MAX_UPSTREAM_LINE: u64 = 4096;
const UPSTREAM_WITHIN: std::time::Duration = std::time::Duration::from_secs(5);

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

/// Where the proxy listens.
enum Listen {
    Tcp(SocketAddr),
    Unix { path: std::path::PathBuf, client_uid: u32 },
}

fn config_from_env() -> Result<(Listen, Config), String> {
    let listen = match (env("PROXY_LISTEN_UNIX"), env("PROXY_CLIENT_UID")) {
        (Some(path), Some(uid)) => {
            Listen::Unix { path: path.into(), client_uid: uid.parse().map_err(|_| "PROXY_CLIENT_UID: not a uid")? }
        }
        (None, None) => {
            let listen = env("PROXY_LISTEN").unwrap_or_else(|| "0.0.0.0:3128".to_owned());
            Listen::Tcp(listen.parse().map_err(|_| format!("PROXY_LISTEN: invalid socket address {listen:?}"))?)
        }
        _ => return Err("PROXY_LISTEN_UNIX and PROXY_CLIENT_UID go together".into()),
    };
    let max_tunnels = match env("PROXY_MAX_TUNNELS") {
        None => 256,
        Some(text) => match text.parse::<usize>() {
            Ok(n) if (1..=65_536).contains(&n) => n,
            _ => return Err(format!("PROXY_MAX_TUNNELS: expected 1..=65536, got {text:?}")),
        },
    };
    let fixture = env("PROXY_BENCH_FIXTURE")
        .map(|spec| BenchFixture::parse(&spec).map_err(|error| format!("PROXY_BENCH_FIXTURE: {error}")))
        .transpose()?;
    let fixture_only = match env("PROXY_FIXTURE_ONLY").as_deref() {
        None => false,
        Some("1") if fixture.is_some() && !upstream_on_stdin()? => true,
        Some("1") => return Err("PROXY_FIXTURE_ONLY needs PROXY_BENCH_FIXTURE and no upstream".into()),
        Some(_) => return Err("PROXY_FIXTURE_ONLY: expected 1".into()),
    };
    let mut config = Config::new(max_tunnels, fixture);
    config.fixture_only = fixture_only;
    Ok((listen, config))
}

/// Whether the upstream comes on standard input.
fn upstream_on_stdin() -> Result<bool, String> {
    match env("PROXY_UPSTREAM_STDIN").as_deref() {
        None => Ok(false),
        Some("1") => Ok(true),
        Some(_) => Err("PROXY_UPSTREAM_STDIN: expected 1".into()),
    }
}

/// Reads the upstream's one JSON line; the error never repeats what was read.
async fn read_upstream() -> Result<Dialer, String> {
    let mut line = Vec::new();
    let mut stdin = tokio::io::stdin().take(MAX_UPSTREAM_LINE + 1);
    let read = stdin.read_to_end(&mut line);
    tokio::time::timeout(UPSTREAM_WITHIN, read)
        .await
        .map_err(|_| "none arrived in time")?
        .map_err(|e| e.to_string())?;
    if line.len() as u64 > MAX_UPSTREAM_LINE || line.last() != Some(&b'\n') {
        return Err("not one line of at most 4096 bytes".into());
    }
    let upstream: egress::Upstream = serde_json::from_slice(&line).map_err(|_| "not an upstream")?;
    Dialer::new(upstream).map_err(|e| e.to_string())
}

/// Binds the socket and gives it its directory's group and mode 0660, so exactly that group can connect.
fn bind_unix(path: &std::path::Path) -> std::io::Result<tokio::net::UnixListener> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let _ = std::fs::remove_file(path);
    let listener = tokio::net::UnixListener::bind(path)?;
    let shared = (|| {
        let gid = std::fs::metadata(path.parent().unwrap_or(std::path::Path::new("/")))?.gid();
        std::os::unix::fs::chown(path, None, Some(gid))?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o660))
    })();
    if let Err(error) = shared {
        // Never leave a socket behind that looks ready but serves nobody.
        let _ = std::fs::remove_file(path);
        return Err(error);
    }
    Ok(listener)
}

fn main() -> ExitCode {
    let (listen, mut config, upstream) = match config_from_env().and_then(|(l, c)| Ok((l, c, upstream_on_stdin()?))) {
        Ok(parsed) => parsed,
        Err(error) => {
            eprintln!("proxy: {error}");
            return ExitCode::from(2);
        }
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("proxy: cannot start the runtime: {error}");
            return ExitCode::FAILURE;
        }
    };
    let code = runtime.block_on(async move {
        let (Ok(mut term), Ok(mut int)) = (signal(SignalKind::terminate()), signal(SignalKind::interrupt())) else {
            eprintln!("proxy: cannot install signal handlers");
            return ExitCode::FAILURE;
        };
        // Through the upstream, names resolve through it too; without one, by the system resolver.
        let resolver: Arc<dyn Resolver> = if upstream {
            let dialer = match read_upstream().await {
                Ok(dialer) => Arc::new(dialer),
                Err(error) => {
                    eprintln!("proxy: the upstream: {error}");
                    return ExitCode::from(2);
                }
            };
            config.upstream = Some(dialer.clone());
            match doh::Doh::new(dialer) {
                Ok(doh) => Arc::new(doh),
                Err(error) => {
                    eprintln!("proxy: the upstream's resolver: {error}");
                    return ExitCode::FAILURE;
                }
            }
        } else {
            Arc::new(SystemResolver)
        };
        let sink = log::stdout_sink();
        let fixture = config.bench_fixture.as_ref().map(|f| format!(", bench fixture {}={}", f.name, f.addr));
        let detail = format!("{} tunnels{}", config.max_tunnels, fixture.unwrap_or_default());
        let serving = match listen {
            Listen::Tcp(address) => match TcpListener::bind(address).await {
                Ok(listener) => {
                    eprintln!("proxy: listening on {address}, {detail}");
                    if upstream {
                        sink("{\"ready\":true}".to_owned());
                    }
                    tokio::spawn(server::serve(listener, config, resolver, sink))
                }
                Err(error) => {
                    eprintln!("proxy: cannot listen on {address}: {error}");
                    return ExitCode::FAILURE;
                }
            },
            Listen::Unix { path, client_uid } => match bind_unix(&path) {
                Ok(listener) => {
                    eprintln!("proxy: listening on {} for uid {client_uid}, {detail}", path.display());
                    if upstream {
                        sink("{\"ready\":true}".to_owned());
                    }
                    tokio::spawn(server::serve_unix(listener, client_uid, config, resolver, sink))
                }
                Err(error) => {
                    eprintln!("proxy: cannot listen on {}: {error}", path.display());
                    return ExitCode::FAILURE;
                }
            },
        };
        tokio::select! {
            _ = serving => ExitCode::FAILURE,
            _ = term.recv() => ExitCode::SUCCESS,
            _ = int.recv() => ExitCode::SUCCESS,
        }
    });
    // Without waiting for the blocking thread of a standard input read that never ended (Tokio cannot cancel it).
    runtime.shutdown_background();
    code
}
