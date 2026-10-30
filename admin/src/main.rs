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
//! first, default `en-US,en`): set both to match where the browsers' traffic leaves, as sites compare them with it (a
//! session through an egress, which the administrator adds in the dashboard, takes that egress's; see `egresses`).
//! `ADMIN_LOCAL_MCP` (`1`, the default outside the demo, or
//! `0`) serves the local MCP socket (see `local`).
//!
//! Three subcommands run beside the server, never starting it: `admin mcp` relays its stdio to the running admin's
//! local MCP socket (see `local`); `admin init` sets the password from one line on its stdin unless another is set
//! (exit 0 when it set it or it was that one already, 3 when another was set, 1 on any failure; see
//! `Auth::initialize`); and `admin reset-second-step` turns the second step at
//! sign-in off, its authenticator app, passkeys and recovery codes, for whoever lost them (exit 0, 1 on any failure;
//! see `second_step::reset`). Each runs where the state directory is, so only someone with access to the server can.

mod agent;
mod agent_api;
mod api;
mod app;
mod assist;
mod auth;
mod coded;
mod controller;
mod deadline;
mod demo;
mod egresses;
mod inject;
mod keys;
mod local;
mod mcp;
mod private;
mod profiles;
mod redact;
mod relay;
mod second_step;
mod server;
mod service;
mod tls;
mod tokens;
mod tools;
mod totp;
mod udp;

use std::net::SocketAddr;
use std::time::Duration;

use display_protocol::session::MAX_SESSIONS;

use crate::app::{App, Config, Locale};
use crate::auth::Auth;
use crate::controller::Controller;

/// The variable `name`, unless it is unset or empty.
fn set(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// Whether `ADMIN_DEMO_SECONDS` holds anything, read or not: a subcommand that the demo has no use for refuses then,
/// rather than take a value it cannot read for none.
fn demo_asked() -> bool {
    std::env::var_os("ADMIN_DEMO_SECONDS").is_some_and(|v| !v.is_empty())
}

fn env(name: &str, default: &str) -> String {
    set(name).unwrap_or_else(|| default.to_owned())
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

/// The exit status of `admin init` when another password was set already.
const SET_ALREADY: i32 = 3;

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

/// `admin reset-second-step`, outside the demo, which has no second step.
fn reset_second_step() -> Result<bool, String> {
    if demo_asked() {
        return Err("the public demo has no second step".into());
    }
    second_step::reset(&state_dir())
}

/// Ends a subcommand with its message on stderr and its exit status: a failure's is 1.
fn finish(name: &str, outcome: Result<(i32, String), String>) -> ! {
    let (code, message) = outcome.unwrap_or_else(|why| (1, why));
    eprintln!("admin {name}: {message}");
    std::process::exit(code)
}

#[tokio::main]
async fn main() {
    // Non-dumpable before anything else: the kernel makes no core dump of it, whatever the host's core pattern (a
    // collector the pattern pipes crashes to gets them at any core limit but 1), and no other process of its uid may
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
        ["reset-second-step"] => finish(
            "reset-second-step",
            reset_second_step().map(|on| match on {
                true => (
                    0,
                    "the second step is off; sign in with the password alone, then turn it on again under Security"
                        .into(),
                ),
                false => (0, "the second step was off already; the password alone signs in".into()),
            }),
        ),
        ["init"] => finish(
            "init",
            init().map(|set| match set {
                auth::Initialized::Set => (0, "the password is set".into()),
                auth::Initialized::Other => {
                    (SET_ALREADY, "another password is set already; it was left as it is".into())
                }
            }),
        ),
        _ => {
            eprintln!("usage: admin [mcp | init | reset-second-step]");
            std::process::exit(2);
        }
    }
    // webauthn-rs traces the credentials and ceremonies it handles: whatever RUST_LOG asks, its crates log warnings at
    // most, by a filter of their own after RUST_LOG's.
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    use tracing_subscriber::{Layer, filter};
    // So do wtransport's, which trace what a peer of the UDP port sends (an address's ticket included) before any
    // ticket is checked; and quinn's log errors only, as it warns of what any peer may send (an Initial with a bad
    // token), and the port answers anyone.
    let webauthn = filter::filter_fn(|meta| {
        let target = meta.target();
        let most = if target.starts_with("quinn") {
            tracing::Level::ERROR
        } else if ["webauthn_rs", "wtransport"].iter().any(|t| target.starts_with(t)) {
            tracing::Level::WARN
        } else {
            return true;
        };
        *meta.level() <= most
    });
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with(tracing_subscriber::fmt::layer().with_filter(webauthn))
        .init();
    if let Err(why) = run().await {
        eprintln!("admin: {why}");
        std::process::exit(1);
    }
}

/// The origins that may frame the demo: up to four `https://host[:port]`, comma-separated, each spelled as
/// `ADMIN_ORIGIN` is, the way a browser reports its parent's origin (the client compares them as they are).
fn frame_ancestors(value: Option<&str>) -> Result<Vec<String>, String> {
    let Some(value) = value else { return Ok(Vec::new()) };
    let origins: Vec<String> = value.split(',').map(|o| o.trim().to_owned()).collect();
    let valid = |o: &str| {
        o.strip_prefix("https://").is_some_and(|host| {
            !host.is_empty()
                && host.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-.:".contains(&b))
        }) && admin_origin(o).is_ok()
    };
    if origins.len() > 4 || !origins.iter().all(|o| valid(o)) {
        let why = "ADMIN_FRAME_ANCESTORS must be up to four https://host[:port] origins, comma-separated, each with a \
                   lowercase host name or IPv4 address (browsers take no IPv6 one there), no default port and nothing \
                   after";
        return Err(why.into());
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

/// `ADMIN_UDP_URL`: where viewers reach the admin's UDP port, `https://host[:port]` spelled as `ADMIN_ORIGIN` is;
/// unset or empty, the direct path is off.
fn udp_url(value: Option<&str>) -> Result<Option<String>, String> {
    value
        .map(|v| {
            admin_origin(v).map_err(|_| format!("ADMIN_UDP_URL must be https://host[:port], like ADMIN_ORIGIN: {v:?}"))
        })
        .transpose()
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
    display_protocol::frame::parse_output_size(value).ok_or_else(|| {
        format!("ADMIN_OUTPUT must be WIDTHxHEIGHT, {}: {value:?}", display_protocol::frame::OUTPUT_SIZE_RULE)
    })
}

async fn run() -> Result<(), String> {
    let origin = admin_origin(
        &std::env::var("ADMIN_ORIGIN").map_err(|_| "ADMIN_ORIGIN is required (e.g. https://localhost:9443)")?,
    )?;
    let listen: SocketAddr = env("ADMIN_LISTEN", "0.0.0.0:9443").parse().map_err(|_| "bad ADMIN_LISTEN")?;
    let hostnames = hostnames(&env("ADMIN_HOSTNAMES", "localhost,127.0.0.1"))?;
    let (width, height) = output_size(&env("ADMIN_OUTPUT", "1280x800"))?;
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
        max_sessions: env("ADMIN_MAX_SESSIONS", "1")
            .parse()
            .ok()
            .filter(|n| (1..=MAX_SESSIONS).contains(n))
            .ok_or(format!("ADMIN_MAX_SESSIONS must be 1-{MAX_SESSIONS}"))?,
        local_mcp: local_mcp(set("ADMIN_LOCAL_MCP").as_deref(), demo.is_some())?,
        demo,
        frame_ancestors: frame_ancestors(set("ADMIN_FRAME_ANCESTORS").as_deref())?,
        udp: udp_url(set("ADMIN_UDP_URL").as_deref())?,
    };
    if config.demo.is_none() && !config.frame_ancestors.is_empty() {
        return Err("only the public demo may be shown in a frame: unset ADMIN_FRAME_ANCESTORS".into());
    }
    if config.demo.is_some() && config.udp.is_some() {
        return Err("the demo has no direct path: unset ADMIN_UDP_URL".into());
    }
    if config.demo.is_some() && config.local_mcp {
        return Err("the demo serves no local MCP socket: unset ADMIN_LOCAL_MCP".into());
    }
    std::fs::create_dir_all(&config.state_dir).map_err(|e| e.to_string())?;
    let tls = tls::server_config(&config.state_dir, &hostnames)?;
    let mut auth =
        Auth::load(&config.state_dir, config.demo.is_none()).map_err(|why| format!("the password: {why}"))?;
    auth.offer_passkeys(&config.origin);
    // The demo has no agent tokens; anywhere else a token file that cannot be read whole stops the start.
    let tokens = if config.demo.is_some() {
        tokens::Tokens::empty(&config.state_dir)
    } else {
        tokens::Tokens::load(&config.state_dir).map_err(|why| format!("agent tokens: {why}"))?
    };
    // So do the saved profiles, outside the demo, which has none.
    let profiles = if config.demo.is_some() {
        profiles::Profiles::empty(&config.state_dir)
    } else {
        profiles::Profiles::load(&config.state_dir).map_err(|why| format!("saved profiles: {why}"))?
    };
    // So do the egresses, outside the demo, which has none.
    let egresses = if config.demo.is_some() {
        egresses::Egresses::empty(&config.state_dir)
    } else {
        egresses::Egresses::load(&config.state_dir).map_err(|why| format!("egresses: {why}"))?
    };
    if let Some(demo) = &config.demo {
        tracing::warn!(seconds = demo.seconds, visits = config.max_sessions, "public demo: one visit per client");
    }
    let controller = Controller::new(env("ADMIN_CONTROLLER", "/run/controller/controller.sock").into());
    let app = App::new(config, auth, controller, tokens, profiles, egresses);
    if let Some(udp) = &app.udp {
        let endpoint = udp::endpoint(udp)?;
        tracing::info!(listen = %udp::LISTEN, origin = udp.origin(), "direct path listening");
        tokio::spawn(udp::serve(app.clone(), endpoint));
    }
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
                expiry.end_login(login, crate::app::ENDED);
            }
            if expiry.config.demo.is_some() {
                demo::end_unwatched(&expiry);
            }
            // While no password is set, off the workers: `admin init` may have set one (read off the lock).
            let pending = expiry.auth.lock().unwrap().pending();
            if let Some(file) = pending {
                let app = expiry.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    let found = Auth::read_hash(&file);
                    app.auth.lock().unwrap().adopt(found);
                })
                .await;
            }
            // Off the async workers and outside the tokens lock: a slow disk never stalls a bearer check.
            let due = expiry.tokens.lock().unwrap().due();
            if let Some(due) = due {
                match tokio::task::spawn_blocking(move || due.write()).await {
                    Ok(Ok(written)) => expiry.tokens.lock().unwrap().written(&written),
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
    use super::{admin_origin, frame_ancestors, hostnames, local_mcp, static_path, udp_url};

    #[test]
    fn the_direct_path_is_off_unless_its_address_is_an_https_origin() {
        assert_eq!(udp_url(None), Ok(None));
        assert_eq!(udp_url(Some("https://203.0.113.7:9443")), Ok(Some("https://203.0.113.7:9443".into())));
        for bad in ["http://203.0.113.7:9443", "https://203.0.113.7:9443/", "https://h:1/media", "203.0.113.7"] {
            assert!(udp_url(Some(bad)).is_err(), "{bad}");
        }
    }

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
    fn only_plain_https_origins_may_frame_the_demo() {
        assert_eq!(frame_ancestors(None), Ok(vec![]));
        assert_eq!(frame_ancestors(Some("https://easi.run")), Ok(vec!["https://easi.run".to_owned()]));
        assert_eq!(frame_ancestors(Some("https://a.run, https://b.run:8443")).map(|o| o.len()), Ok(2));
        for bad in [
            "http://easi.run",
            "https://easi.run/",
            "https://",
            "*",
            "https://easi.run; script-src *",
            "https://A.run",
            "https://easi.run:443",
        ] {
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
