//! easi.run egress proxy: the browser container's only route to the internet.
//!
//! It accepts `CONNECT host:port` (ports 80 and 443) and absolute-form plain HTTP on port 80, resolves each name
//! itself exactly once, refuses the request when any answer is a non-public address, and dials a vetted address.
//!
//! Configuration comes from the environment:
//! - `PROXY_LISTEN` — the listen address, default `0.0.0.0:3128`.
//! - `PROXY_LISTEN_UNIX` and `PROXY_CLIENT_UID` — instead, a Unix socket that only that uid may use: the browser has
//!   no network, and the browser launcher's loopback forwarder brings Chromium's proxy connections here.
//! - `PROXY_BENCH_FIXTURE` — `name=ipv4`, set only by the `bench` profile: that exact name maps to that exact
//!   address without DNS. Absent (or empty) in production, which has no exception.
//! - `PROXY_FIXTURE_ONLY` — `1`, the demo's walled garden: every destination goes to that fixture and nowhere else;
//!   it needs `PROXY_BENCH_FIXTURE` and no upstream.
//! - `PROXY_UPSTREAM_STDIN` — `1` when the session's traffic leaves through the deployment's upstream proxy: it arrives
//!   as one JSON line on standard input (never in the environment, which `docker inspect` shows). Without a valid one
//!   within five seconds the proxy exits; with one, every connection and every name resolution goes through it, and
//!   once listening the proxy prints `{"ready":true}` as its first line on standard output, which the controller
//!   awaits.
//! - `PROXY_CHECK` — `1`, alone: instead of serving, the proxy checks the upstream that arrives on standard input as
//!   above (see `check`), prints the outcome as one JSON line on standard output, and exits 0.

mod check;
mod doh;
mod log;
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

/// Concurrent requests; excess connections get 503.
const MAX_TUNNELS: usize = 256;
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
    let fixture = env("PROXY_BENCH_FIXTURE")
        .map(|spec| BenchFixture::parse(&spec).map_err(|error| format!("PROXY_BENCH_FIXTURE: {error}")))
        .transpose()?;
    let fixture_only = match env("PROXY_FIXTURE_ONLY").as_deref() {
        None => false,
        Some("1") if fixture.is_some() && !upstream_on_stdin()? => true,
        Some("1") => return Err("PROXY_FIXTURE_ONLY needs PROXY_BENCH_FIXTURE and no upstream".into()),
        Some(_) => return Err("PROXY_FIXTURE_ONLY: expected 1".into()),
    };
    let mut config = Config::new(MAX_TUNNELS, fixture);
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

/// Whether this run checks an upstream rather than serving (`PROXY_CHECK=1`), which nothing else configures.
fn checking() -> Result<bool, String> {
    match env("PROXY_CHECK").as_deref() {
        None => Ok(false),
        Some("1") => {
            let others = std::env::vars()
                .any(|(name, value)| name.starts_with("PROXY_") && name != "PROXY_CHECK" && !value.is_empty());
            if others { Err("PROXY_CHECK goes alone".into()) } else { Ok(true) }
        }
        Some(_) => Err("PROXY_CHECK: expected 1".into()),
    }
}

/// The check: the upstream from standard input, its outcome on standard output.
fn check_upstream() -> Result<ExitCode, (ExitCode, String)> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| (ExitCode::FAILURE, format!("cannot start the runtime: {error}")))?;
    let checked = runtime.block_on(async {
        let dialer = read_upstream().await.map_err(|error| (ExitCode::from(2), format!("the upstream: {error}")))?;
        let dialer = Arc::new(dialer);
        let probe = check::Probe::new(dialer.clone())
            .map_err(|error| (ExitCode::FAILURE, format!("the check's TLS: {error}")))?;
        Ok(check::check(&dialer, &probe).await)
    });
    runtime.shutdown_background();
    let outcome = checked?;
    use std::io::Write as _;
    let mut out = std::io::stdout().lock();
    let line = serde_json::to_string(&outcome).map_err(|e| (ExitCode::FAILURE, e.to_string()))?;
    writeln!(out, "{line}").and_then(|()| out.flush()).map_err(|e| (ExitCode::FAILURE, e.to_string()))?;
    Ok(ExitCode::SUCCESS)
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
    match run() {
        Ok(code) => code,
        Err((code, why)) => {
            eprintln!("proxy: {why}");
            code
        }
    }
}

/// Serves until a signal (success) or the server's end (failure); an error is the exit code, 2 for a configuration it
/// refuses, and its message.
fn run() -> Result<ExitCode, (ExitCode, String)> {
    let failure = |why: String| (ExitCode::FAILURE, why);
    let refused = |why: String| (ExitCode::from(2), why);
    // Non-dumpable before anything else: the kernel makes no core dump of it, whatever the host's core pattern (a
    // collector the pattern pipes crashes to gets them at any core limit but 1), and no other process of its uid may
    // read its memory.
    rustix::process::set_dumpable_behavior(rustix::process::DumpableBehavior::NotDumpable)
        .map_err(|why| failure(format!("cannot make itself non-dumpable: {why}")))?;
    if checking().map_err(refused)? {
        return check_upstream();
    }
    let (listen, mut config) = config_from_env().map_err(refused)?;
    let upstream = upstream_on_stdin().map_err(refused)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| failure(format!("cannot start the runtime: {error}")))?;
    let served = runtime.block_on(async move {
        let (Ok(mut term), Ok(mut int)) = (signal(SignalKind::terminate()), signal(SignalKind::interrupt())) else {
            return Err(failure("cannot install signal handlers".into()));
        };
        // Through the upstream, names resolve through it too; without one, by the system resolver.
        let resolver: Arc<dyn Resolver> = if upstream {
            let dialer = Arc::new(read_upstream().await.map_err(|error| refused(format!("the upstream: {error}")))?);
            config.upstream = Some(dialer.clone());
            Arc::new(doh::Doh::new(dialer).map_err(|error| failure(format!("the upstream's resolver: {error}")))?)
        } else {
            Arc::new(SystemResolver)
        };
        let sink = log::stdout_sink();
        let fixture = config.bench_fixture.as_ref().map(|f| format!(", bench fixture {}={}", f.name, f.addr));
        let detail = format!("{} tunnels{}", config.max_tunnels, fixture.unwrap_or_default());
        // Not yet polled: the ready line goes out before any request's.
        let serve: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> = match listen {
            Listen::Tcp(address) => {
                let listener = TcpListener::bind(address)
                    .await
                    .map_err(|error| failure(format!("cannot listen on {address}: {error}")))?;
                eprintln!("proxy: listening on {address}, {detail}");
                Box::pin(server::serve(listener, config, resolver, sink.clone()))
            }
            Listen::Unix { path, client_uid } => {
                let listener = bind_unix(&path)
                    .map_err(|error| failure(format!("cannot listen on {}: {error}", path.display())))?;
                eprintln!("proxy: listening on {} for uid {client_uid}, {detail}", path.display());
                Box::pin(server::serve_unix(listener, client_uid, config, resolver, sink.clone()))
            }
        };
        if upstream {
            sink("{\"ready\":true}".to_owned());
        }
        let serving = tokio::spawn(serve);
        Ok(tokio::select! {
            _ = serving => ExitCode::FAILURE,
            _ = term.recv() => ExitCode::SUCCESS,
            _ = int.recv() => ExitCode::SUCCESS,
        })
    });
    // Without waiting for the blocking thread of a standard input read that never ended (Tokio cannot cancel it).
    runtime.shutdown_background();
    served
}
