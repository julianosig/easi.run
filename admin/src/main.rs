//! The easi.run admin application: one person's HTTPS interface to start browser sessions, watch them, take
//! and give back control, run the scripted agent, and read live statistics.
//!
//! Environment: `ADMIN_ORIGIN` (required, the exact `https://host:port` the person's browser uses), `ADMIN_LISTEN`
//! (default `0.0.0.0:9443`), `ADMIN_HOSTNAMES` (certificate names, default `localhost,127.0.0.1`), `ADMIN_STATE`
//! (default `/var/lib/admin`), `ADMIN_STATIC` (default `/srv/admin`), `ADMIN_STREAMS` (default `/run/streams`),
//! `ADMIN_CONTROLLER` (default `/run/controller/controller.sock`), `ADMIN_OUTPUT` (default `1280x800`),
//! `ADMIN_MAX_SESSIONS` (browser sessions at once, 1-16, default 1, which fits a 4 GB server: each holds up to 2 GiB
//! for its browser by default),
//! `ADMIN_TIMEZONE` (the browsers' IANA time zone, default `UTC`) and `ADMIN_LANGUAGES` (their languages, preferred
//! first, default `en-US,en`): set both to match where the browsers' traffic leaves, as sites compare them with it.
//! `ADMIN_EGRESS_FILE` (optional) names the deployment's upstream proxies a session may leave through instead, each
//! with its own time zone and languages (see `egresses`). `ADMIN_LOCAL_MCP` (`1`, the default outside the demo, or
//! `0`) serves the local MCP socket (see `local`).
//!
//! Three subcommands run beside the server, never starting it: `admin mcp` relays its stdio to the running admin's
//! local MCP socket (see `local`); `admin setup` prints the one-time link to the admin's first screen, where the
//! person creates the password, on stdout, and how long it lasts on stderr (exit 0, 3 when a password is set already, 1 on any failure; see `Auth::setup_token`);
//! and `admin init` sets the password from one line on its stdin unless one is set (exit 0 when it set it, 3 when one
//! was set already, 1 on any failure; see `Auth::initialize`).

mod agent;
mod agent_api;
mod api;
mod app;
mod auth;
mod controller;
mod deadline;
mod demo;
mod egresses;
mod line;
mod local;
mod mcp;
mod private;
mod redact;
mod relay;
mod server;
mod service;
mod tls;
mod tokens;
mod tools;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::Duration;

use crate::app::{App, Config, Locale};
use crate::auth::Auth;
use crate::controller::Controller;

/// The most browser sessions `ADMIN_MAX_SESSIONS` may allow: the controller's own hard maximum, which a controller test
/// holds equal to this one.
const MAX_SESSIONS: usize = 16;

fn env(name: &str, default: &str) -> String {
    std::env::var(name).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| default.to_owned())
}

/// `ADMIN_STATE`: the state directory, the same one for the server and every subcommand, which find each other's files
/// there.
fn state_dir() -> std::path::PathBuf {
    env("ADMIN_STATE", "/var/lib/admin").into()
}

/// `ADMIN_TIMEZONE` and `ADMIN_LANGUAGES`, refused here rather than at every session the controller would refuse.
fn browser_locale() -> Result<Locale, String> {
    let timezone = env("ADMIN_TIMEZONE", "UTC");
    if !locale::valid_timezone(&timezone) {
        return Err(format!("ADMIN_TIMEZONE is not an IANA time zone such as America/Sao_Paulo: {timezone:?}"));
    }
    let languages: Vec<String> = env("ADMIN_LANGUAGES", "en-US,en").split(',').map(|l| l.trim().to_owned()).collect();
    if !locale::valid_languages(&languages) {
        return Err(format!("ADMIN_LANGUAGES must be 1 to {} tags such as pt-BR,en", locale::MAX_LANGUAGES));
    }
    Ok(Locale { timezone, languages })
}

/// The exit status of `admin init` and `admin setup` when a password (another one, for `admin init`) was set
/// already.
const SET_ALREADY: i32 = 3;
/// How long `admin setup` waits for the running admin to follow a token about to expire (or expired) with a new
/// one, which it does within seconds.
const SETUP_WAIT: Duration = Duration::from_secs(10);

/// `admin init`: the password, one line on stdin (its newline dropped), set unless one is.
fn init() -> Result<auth::Initialized, String> {
    use std::io::Read;
    let mut line = Vec::new();
    // A little more than the longest password allowed, in bytes, so a longer one is refused rather than cut.
    std::io::stdin()
        .take(4 * auth::MAX_PASSWORD as u64 + 2)
        .read_to_end(&mut line)
        .map_err(|e| format!("reading stdin: {e}"))?;
    let line = String::from_utf8(line).map_err(|_| "the password must be UTF-8")?;
    let password = line.strip_suffix('\n').unwrap_or(&line);
    if password.contains(['\n', '\r']) {
        return Err("the password is one line".into());
    }
    Auth::initialize(&state_dir(), password)
}

/// `admin setup`: the admin page's address with the running admin's enrollment token in its fragment, which a
/// browser never sends to any server, and how long it stays valid.
fn setup() -> Result<Option<(String, Duration)>, String> {
    if std::env::var_os("ADMIN_DEMO_SECONDS").is_some_and(|v| !v.is_empty()) {
        return Err("the public demo has no password to set".into());
    }
    let origin = admin_origin(&std::env::var("ADMIN_ORIGIN").map_err(|_| "ADMIN_ORIGIN is not set")?)?;
    let path = static_path(std::env::var("ADMIN_STATIC_PATH").ok().filter(|v| !v.is_empty()).as_deref())?;
    Ok(match Auth::setup_token(&state_dir(), SETUP_WAIT)? {
        auth::SetupLink::Token(token, left) => Some((format!("{}#setup={token}", service::page(&origin, &path)), left)),
        auth::SetupLink::Set => None,
    })
}

#[tokio::main]
async fn main() {
    // Non-dumpable before anything else: the kernel makes no core dump of it, whatever the host's core pattern (a
    // collector the pattern pipes crashes to gets them whatever the core limit), and no other process of its uid may
    // read its memory.
    if let Err(why) = rustix::process::set_dumpable_behavior(rustix::process::DumpableBehavior::NotDumpable) {
        eprintln!("admin: cannot make itself non-dumpable: {why}");
        std::process::exit(1);
    }
    // The subcommands log nothing: `admin mcp`'s stdout carries only MCP messages, and a failure goes to stderr.
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        [] => {}
        ["mcp"] => {
            let socket = state_dir().join(local::SOCKET);
            let relayed = local::relay(&socket).await;
            // Not a return: the runtime would wait for the thread still reading stdin.
            std::process::exit(match relayed {
                Ok(()) => 0,
                Err(why) => {
                    eprintln!("admin mcp: {why}");
                    1
                }
            });
        }
        ["setup"] => std::process::exit(match setup() {
            // The link alone on stdout, for scripts (the installer reads it); its lifetime, rounded down, for the person.
            Ok(Some((link, left))) => {
                println!("{link}");
                eprintln!(
                    "admin setup: the link sets the password once, within the next {} minutes",
                    left.as_secs() / 60
                );
                0
            }
            Ok(None) => {
                eprintln!("admin setup: a password is set already");
                SET_ALREADY
            }
            Err(why) => {
                eprintln!("admin setup: {why}");
                1
            }
        }),
        ["init"] => std::process::exit(match init() {
            Ok(auth::Initialized::Set) => {
                eprintln!("admin init: the password is set");
                0
            }
            Ok(auth::Initialized::Other) => {
                eprintln!("admin init: another password is set already; it was left as it is");
                SET_ALREADY
            }
            Err(why) => {
                eprintln!("admin init: {why}");
                1
            }
        }),
        _ => {
            eprintln!("usage: admin [mcp | setup | init]");
            std::process::exit(2);
        }
    }
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    if let Err(why) = run().await {
        eprintln!("admin: {why}");
        std::process::exit(1);
    }
}

/// The origins that may frame the demo: up to four `https://host[:port]`, comma-separated.
fn frame_ancestors(value: Option<&str>) -> Result<Vec<String>, String> {
    let Some(value) = value else { return Ok(Vec::new()) };
    let origins: Vec<String> = value.split(',').map(|o| o.trim().to_owned()).collect();
    let valid = |o: &str| {
        o.strip_prefix("https://").is_some_and(|host| {
            !host.is_empty()
                && host.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-.:".contains(&b))
        })
    };
    if origins.len() > 4 || !origins.iter().all(|o| valid(o)) {
        return Err("ADMIN_FRAME_ANCESTORS must be up to four https://host[:port] origins, comma-separated".into());
    }
    Ok(origins)
}

/// `ADMIN_HOSTNAMES`: the names the generated certificate is for, comma-separated, each an IP address or a DNS name
/// (a leading `*.` allowed), spaces around them ignored and letters lowercased. An empty, repeated or malformed one is
/// refused here: it would put a name no browser matches in the certificate, and the record kept beside it would keep
/// that certificate from then on.
fn hostnames(value: &str) -> Result<Vec<String>, String> {
    let dns = |name: &str| {
        let name = name.strip_prefix("*.").unwrap_or(name);
        name.len() <= 253
            && name.split('.').all(|label| {
                (1..=63).contains(&label.len())
                    && !label.starts_with('-')
                    && !label.ends_with('-')
                    && label.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            })
    };
    let mut names: Vec<String> = Vec::new();
    for name in value.split(',').map(|n| n.trim().to_ascii_lowercase()) {
        if !(name.parse::<std::net::IpAddr>().is_ok() || dns(&name)) || names.contains(&name) {
            return Err(format!(
                "ADMIN_HOSTNAMES must be distinct IP addresses or DNS names, comma-separated: {name:?}"
            ));
        }
        names.push(name);
    }
    Ok(names)
}

/// `ADMIN_ORIGIN`: exactly the `Origin` a browser sends for the admin, `https://host[:port]`. The browser serializes it
/// as the URL standard does (host lowercased, IPv4 shorthand expanded, IPv6 bracketed, no default port, nothing after
/// the authority); any other spelling would never match, refusing every POST and WebSocket, so it is refused here, at
/// startup.
fn admin_origin(value: &str) -> Result<String, String> {
    let serialized = url::Url::parse(value)
        .ok()
        .filter(|u| u.scheme() == "https" && u.username().is_empty() && u.password().is_none())
        .map(|u| u.origin().ascii_serialization());
    if serialized.as_deref() != Some(value) {
        return Err(format!(
            "ADMIN_ORIGIN must be https://host[:port] with a lowercase host, no default port and nothing after: {value:?}"
        ));
    }
    Ok(value.to_owned())
}

/// `ADMIN_STATIC_PATH`: `/`, or one lowercase segment that is not one of the admin's own (`/api`, `/ws`, `/sdk`,
/// `/mcp`).
fn static_path(value: Option<&str>) -> Result<String, String> {
    let Some(value) = value.filter(|v| *v != "/") else { return Ok("/".into()) };
    let segment = value.strip_prefix('/').unwrap_or_default();
    let valid = (1..=32).contains(&segment.len())
        && segment.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !["api", "ws", "sdk", "mcp"].contains(&segment);
    if !valid {
        return Err("ADMIN_STATIC_PATH must be / or /name (a-z, 0-9, -; not /api, /ws, /sdk or /mcp)".into());
    }
    Ok(value.to_owned())
}

/// `ADMIN_LOCAL_MCP`: `1` or `0`; unset, on outside the demo and off in it.
fn local_mcp(value: Option<&str>, demo: bool) -> Result<bool, String> {
    match value {
        None => Ok(!demo),
        Some("1") => Ok(true),
        Some("0") => Ok(false),
        Some(other) => Err(format!("ADMIN_LOCAL_MCP must be 1 or 0: {other:?}")),
    }
}

/// `ADMIN_OUTPUT`: `WIDTHxHEIGHT`, within what the protocol's video carries
/// ([`display_protocol::frame::output_size_ok`]). The controller refuses every session outside the same rule
/// (`controller/src/request.rs`, `validate`); checked here once, at startup, rather than at every session.
fn output_size(value: &str) -> Result<(u32, u32), String> {
    let size = value.split_once('x').and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)));
    match size {
        Some((w, h)) if display_protocol::frame::output_size_ok(w, h) => Ok((w, h)),
        _ => Err(format!("ADMIN_OUTPUT must be WIDTHxHEIGHT, both even, within 320x240 and 1920x1080: {value:?}")),
    }
}

async fn run() -> Result<(), String> {
    let origin = admin_origin(
        &std::env::var("ADMIN_ORIGIN").map_err(|_| "ADMIN_ORIGIN is required (e.g. https://localhost:9443)")?,
    )?;
    let listen: SocketAddr = env("ADMIN_LISTEN", "0.0.0.0:9443").parse().map_err(|_| "bad ADMIN_LISTEN")?;
    let hostnames = hostnames(&env("ADMIN_HOSTNAMES", "localhost,127.0.0.1"))?;
    let (width, height) = output_size(&env("ADMIN_OUTPUT", "1280x800"))?;
    let set = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
    let demo = demo::Demo::from_env(set("ADMIN_DEMO_SECONDS"), set("ADMIN_DEMO_SITES"))?;
    let config = Config {
        origin,
        state_dir: state_dir(),
        static_dir: env("ADMIN_STATIC", "/srv/admin").into(),
        static_path: static_path(set("ADMIN_STATIC_PATH").as_deref())?,
        sdk_dir: env("ADMIN_SDK", "/srv/sdk").into(),
        streams_dir: env("ADMIN_STREAMS", "/run/streams").into(),
        width,
        height,
        locale: browser_locale()?,
        egresses: match std::env::var_os("ADMIN_EGRESS_FILE").filter(|p| !p.is_empty()) {
            Some(path) => egresses::load(path.as_ref(), rustix::process::getuid().as_raw())
                .map_err(|why| format!("ADMIN_EGRESS_FILE: {why}"))?,
            None => Vec::new(),
        },
        max_sessions: env("ADMIN_MAX_SESSIONS", "1")
            .parse()
            .ok()
            .filter(|n| (1..=MAX_SESSIONS).contains(n))
            .ok_or(format!("ADMIN_MAX_SESSIONS must be 1-{MAX_SESSIONS}"))?,
        local_mcp: local_mcp(set("ADMIN_LOCAL_MCP").as_deref(), demo.is_some())?,
        demo,
        frame_ancestors: frame_ancestors(set("ADMIN_FRAME_ANCESTORS").as_deref())?,
    };
    if config.demo.is_none() && !config.frame_ancestors.is_empty() {
        return Err("only the public demo may be shown in a frame: unset ADMIN_FRAME_ANCESTORS".into());
    }
    if config.demo.is_some() && !config.egresses.is_empty() {
        return Err("the demo uses no egress: unset ADMIN_EGRESS_FILE".into());
    }
    if config.demo.is_some() && config.local_mcp {
        return Err("the demo serves no local MCP socket: unset ADMIN_LOCAL_MCP".into());
    }
    std::fs::create_dir_all(&config.state_dir).map_err(|e| e.to_string())?;
    let tls = tls::server_config(&config.state_dir, &hostnames)?;
    let auth = Auth::load(&config.state_dir, config.demo.is_none()).map_err(|why| format!("the password: {why}"))?;
    // The demo has no agent tokens; anywhere else a token file that cannot be read whole stops the start.
    let tokens = if config.demo.is_some() {
        tokens::Tokens::empty(&config.state_dir)
    } else {
        tokens::Tokens::load(&config.state_dir).map_err(|why| format!("agent tokens: {why}"))?
    };
    if let Some(demo) = &config.demo {
        tracing::warn!(seconds = demo.seconds, visits = config.max_sessions, "public demo: one visit per client");
    }
    let controller = Controller::new(env("ADMIN_CONTROLLER", "/run/controller/controller.sock").into());
    let app = Arc::new(App {
        auth: std::sync::Mutex::new(auth),
        controller,
        sessions: std::sync::Mutex::new(HashMap::new()),
        starting: std::sync::atomic::AtomicUsize::new(0),
        next_viewer: AtomicU64::new(0),
        downloads: Arc::default(),
        demo_visits: demo::Visits::default(),
        tokens: std::sync::Mutex::new(tokens),
        config,
    });
    if app.config.demo.is_some() {
        // The demo opens only once whatever an earlier run left is confirmed gone.
        demo::open(&app);
    } else {
        // Before serving: a cleanup after a session started would remove that session too.
        app.clean_up_earlier_runs().await;
    }
    if app.config.local_mcp {
        let socket = app.config.state_dir.join(local::SOCKET);
        let listener = local::bind(&socket).map_err(|why| format!("the local MCP socket: {why}"))?;
        tracing::info!(socket = %socket.display(), "local MCP socket listening");
        tokio::spawn(local::serve(listener, app.clone()));
    }
    let expiry = app.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        loop {
            tick.tick().await;
            let expired = expiry.auth.lock().unwrap().expire();
            for login in expired {
                tracing::info!("a login expired; its streams are closed");
                expiry.end_login(login);
            }
            if expiry.config.demo.is_some() {
                demo::end_unwatched(&expiry);
            }
            // While no password is set, off the workers: `admin init` may have set one (read off the lock), and
            // otherwise a token that expired gives way to a new one, so a setup link always works.
            let pending = expiry.auth.lock().unwrap().pending();
            if let Some(file) = pending {
                let app = expiry.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    let found = Auth::read_hash(&file);
                    let mut auth = app.auth.lock().unwrap();
                    auth.adopt(found);
                    auth.renew();
                })
                .await;
            }
            // Off the async workers and outside the tokens lock: a slow disk never stalls a bearer check.
            let due = expiry.tokens.lock().unwrap().due();
            if let Some(due) = due {
                match tokio::task::spawn_blocking(move || due.write()).await {
                    Ok(Ok(written)) => expiry.tokens.lock().unwrap().forget(&written),
                    Ok(Err(why)) => tracing::warn!(%why, "the tokens file could not be written"),
                    Err(_) => {}
                }
            }
        }
    });

    let router = api::router(app);
    tracing::info!(%listen, "admin listening");
    server::serve(listen, tls, router).await.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::{admin_origin, frame_ancestors, hostnames, local_mcp, output_size, static_path};

    #[test]
    fn the_local_mcp_socket_is_on_by_default_outside_the_demo_only() {
        assert_eq!((local_mcp(None, false), local_mcp(None, true)), (Ok(true), Ok(false)));
        assert_eq!((local_mcp(Some("0"), false), local_mcp(Some("1"), true)), (Ok(false), Ok(true)));
        for bad in ["yes", "true", "01", " 1"] {
            assert!(local_mcp(Some(bad), false).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_origin_is_spelled_as_a_browser_sends_it() {
        for good in [
            "https://localhost:9443",
            "https://easi.run",
            "https://127.0.0.1:8443",
            "https://a-b.example.com",
            "https://[::1]:9443",
        ] {
            assert_eq!(admin_origin(good), Ok(good.to_owned()));
        }
        for bad in [
            "http://localhost:9443",
            "https://localhost:9443/",
            "https://localhost:9443/admin",
            "https://Localhost:9443",
            "https://localhost:443",
            "https://localhost:",
            "https://localhost:09443",
            "https://localhost:65536",
            "https://localhost:9443?x",
            "https://user@localhost",
            "https://127.1",
            "https://[0:0::1]:9443",
            "https://",
            "localhost:9443",
        ] {
            assert!(admin_origin(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn certificate_names_are_trimmed_and_each_must_be_one_name() {
        assert_eq!(
            hostnames("localhost, Box.LAN ,127.0.0.1,::1,*.lan").unwrap(),
            ["localhost", "box.lan", "127.0.0.1", "::1", "*.lan"]
        );
        for bad in
            ["localhost,", "localhost,,box", " ", "a b", "box.lan,BOX.lan", "-a.lan", "a..lan", "[::1]", "a.lan."]
        {
            assert!(hostnames(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_output_size_follows_the_protocol_s_rule() {
        assert_eq!(output_size("1280x800"), Ok((1280, 800)));
        for bad in ["1281x800", "1922x1080", "1280", "x800", "-2x4"] {
            assert!(output_size(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn only_plain_https_origins_may_frame_the_demo() {
        assert_eq!(frame_ancestors(None), Ok(vec![]));
        assert_eq!(frame_ancestors(Some("https://easi.run")), Ok(vec!["https://easi.run".to_owned()]));
        assert_eq!(frame_ancestors(Some("https://a.run, https://b.run:8443")).map(|o| o.len()), Ok(2));
        for bad in
            ["http://easi.run", "https://easi.run/", "https://", "*", "https://easi.run; script-src *", "https://A.run"]
        {
            assert!(frame_ancestors(Some(bad)).is_err(), "{bad}");
        }
        assert!(frame_ancestors(Some("https://a,https://b,https://c,https://d,https://e")).is_err());
    }

    #[test]
    fn the_web_client_sits_at_the_root_or_under_one_path_of_its_own() {
        assert_eq!(static_path(None), Ok("/".to_owned()));
        assert_eq!(static_path(Some("/")), Ok("/".to_owned()));
        assert_eq!(static_path(Some("/demo")), Ok("/demo".to_owned()));
        for bad in ["demo", "/demo/", "/a/b", "/Demo", "/api", "/ws", "/sdk", "/mcp"] {
            assert!(static_path(Some(bad)).is_err(), "{bad}");
        }
    }
}
