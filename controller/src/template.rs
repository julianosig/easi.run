//! The fixed per-session templates of `docs/topology.md`, turned into Docker Engine API bodies.
//!
//! Nothing here talks to Docker. Every container body passes `guard::check_container` before it is returned, and the
//! tests below prove that every template does and that the guard refuses each way of widening one.

use std::collections::HashMap;

use bollard::models::{
    ContainerCreateBody, EndpointSettings, HostConfig, HostConfigCgroupnsModeEnum, HostConfigLogConfig, Mount,
    MountType, MountVolumeOptions, NetworkCreateRequest, NetworkingConfig, ResourcesUlimits, VolumeCreateRequest,
};

use crate::guard::check_container;
use crate::image::Images;
use crate::request::SessionId;

pub const LABEL_SESSION: &str = "run.easi.session";
pub const LABEL_ROLE: &str = "run.easi.role";
/// Which installation a resource belongs to. Every name starts with the instance and every cleanup filters on it, so
/// two installations on one Docker Engine (a development stack and the public demo) never see each other's sessions.
pub const LABEL_INSTANCE: &str = "run.easi.instance";

/// The single exec the controller ever creates.
pub const AGENT_EXEC: &str = "/usr/local/bin/agent-mcp";

pub(crate) const UID_BROWSER: u32 = 10001;
pub(crate) const UID_DISPLAY: u32 = 10002;
pub(crate) const UID_PROXY: u32 = 10003;
pub(crate) const UID_ADMIN: u32 = 10004;
const UID_AGENT: u32 = 10005;
pub(crate) const GID_WAYLAND: u32 = 10011;
pub(crate) const GID_BUS: u32 = 10012;
pub(crate) const GID_AGENT_SOCKETS: u32 = 10013;
pub(crate) const GID_STREAMS: u32 = 10014;
pub(crate) const GID_PROXY: u32 = 10016;

pub(crate) const MIB: i64 = 1024 * 1024;

/// Paths inside `display`; `display/src/main.rs` reads them from its environment.
pub(crate) const DISPLAY_BROWSER_DIR: &str = "/run/display/browser";
pub(crate) const DISPLAY_AGENT_DIR: &str = "/run/display/agent";
pub(crate) const DISPLAY_STREAM_DIR: &str = "/run/display/stream";
/// Paths inside `browser` (and the bus directory inside `agent`, at the same path so the addresses the buses
/// publish resolve in both).
pub(crate) const BROWSER_WAYLAND_DIR: &str = "/run/browser/wayland";
pub(crate) const BUS_DIR: &str = "/run/browser/bus";
/// The privileged Wayland and injection sockets inside `agent`.
pub(crate) const AGENT_DIR: &str = "/run/agent";
/// The proxy's socket: in `proxy`, and (read-only) in `browser`.
pub(crate) const PROXY_DIR: &str = "/run/proxy";
pub(crate) const BROWSER_PROXY_DIR: &str = "/run/browser/proxy";

const NOEXEC_TMPFS: &str = "rw,noexec,nosuid,nodev";

/// Everything the templates need besides the request: fixed at start from the environment.
#[derive(Clone, Debug)]
pub struct Fixed {
    /// This installation's name (`CONTROLLER_INSTANCE`): the prefix of every resource name and the value of
    /// [`LABEL_INSTANCE`].
    pub instance: String,
    pub images: Images,
    /// The browser container's CPU and memory limits.
    pub browser: BrowserLimits,
    /// The browser seccomp profile, passed inline as `seccomp=<json>`.
    pub browser_seccomp: String,
    /// The long-lived `streams` volume; each display sees only its session's subpath.
    pub streams_volume: String,
    /// The benchmark fixture host, in the bench profile only.
    pub bench: Option<Bench>,
}

/// The browser container's limits. The defaults are the measured balance: 2 CPUs lift a full-screen repaint from 27 to
/// 32 fps, the measured step with the largest marginal gain (3 CPUs add 3 fps for 0.7 CPU more), while scrolling, animation,
/// video, and a 1080p YouTube video do not change with the browser's CPUs. A limit is a ceiling, not a reservation.
/// 2 GiB holds heavy pages; a YouTube video takes about 270 MiB.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BrowserLimits {
    pub cpus_milli: i64,
    pub memory_mib: i64,
}

impl Default for BrowserLimits {
    fn default() -> BrowserLimits {
        BrowserLimits { cpus_milli: 2000, memory_mib: 2048 }
    }
}

impl BrowserLimits {
    /// Limits from `CPUS` (a decimal number of CPUs, 0.5 to 16) and `MEMORY_MIB` (1024 to 65536), each defaulting when
    /// absent.
    pub fn parse(cpus: Option<&str>, memory_mib: Option<&str>) -> Result<BrowserLimits, String> {
        let mut limits = BrowserLimits::default();
        if let Some(text) = cpus {
            let value: f64 = text.trim().parse().map_err(|_| format!("browser CPUs must be a number, not {text:?}"))?;
            if !(0.5..=16.0).contains(&value) {
                return Err(format!("browser CPUs must be 0.5 to 16, not {text:?}"));
            }
            limits.cpus_milli = (value * 1000.0).round() as i64;
        }
        if let Some(text) = memory_mib {
            let value: i64 = text.trim().parse().map_err(|_| format!("browser memory must be MiB, not {text:?}"))?;
            if !(1024..=65536).contains(&value) {
                return Err(format!("browser memory must be 1024 to 65536 MiB, not {text:?}"));
            }
            limits.memory_mib = value;
        }
        Ok(limits)
    }
}

/// The bench profile's one exception: the session's proxy joins the fixture network and admits exactly the fixture
/// host's name and address, and Chromium accepts exactly the fixture's certificate.
#[derive(Clone, Debug)]
pub struct Bench {
    pub network: String,
    /// `name=ipv4`, passed to the proxy as `PROXY_BENCH_FIXTURE`.
    pub fixture: String,
    /// Where the fixture host publishes its certificate's SPKI SHA-256 (base64).
    pub spki_file: std::path::PathBuf,
    /// The CPU lanes a benchmark may place a session in; none outside a measured benchmark.
    pub lanes: Vec<Lane>,
    /// The public demo's walled garden (`CONTROLLER_BENCH_FIXTURE_ONLY`): every session proxy sends every destination to
    /// the fixture and nowhere else, and the session's networks are internal, so not even a proxy fault reaches out.
    pub fixture_only: bool,
}

/// One benchmark lane: the CPUs the browser and the display run on, and the memory node of both, as cpuset lists
/// (`2,50`). Placement only narrows where a session runs; its CPU quota stays the template's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lane {
    pub browser: String,
    pub display: String,
    pub mems: String,
}

impl Lane {
    /// One lane from `BROWSER/DISPLAY/MEMS`, each a cpuset list of numbers, ranges, and commas.
    pub fn parse(text: &str) -> Result<Lane, String> {
        let list = |s: &str| {
            !s.is_empty()
                && s.len() <= 64
                && s.split(',').all(|part| {
                    let mut ends = part.splitn(2, '-');
                    ends.all(|n| !n.is_empty() && n.len() <= 4 && n.bytes().all(|b| b.is_ascii_digit()))
                })
        };
        match text.split('/').collect::<Vec<_>>()[..] {
            [browser, display, mems] if list(browser) && list(display) && list(mems) => {
                Ok(Lane { browser: browser.into(), display: display.into(), mems: mems.into() })
            }
            _ => Err(format!("a bench lane is BROWSER/DISPLAY/MEMS cpuset lists, not {text:?}")),
        }
    }
}

/// Where a role of this session runs: `(cpus, mems)` for the browser and the display of a session placed in a bench
/// lane, `None` otherwise. A lane the profile does not declare is an error.
pub fn placement(fixed: &Fixed, spec: &Spec, role: Role) -> Result<Option<(String, String)>, String> {
    let Some(index) = spec.lane else { return Ok(None) };
    let lane = fixed
        .bench
        .as_ref()
        .and_then(|b| b.lanes.get(usize::from(index)))
        .ok_or_else(|| format!("refused: no bench lane {index}"))?;
    Ok(match role {
        Role::Browser => Some((lane.browser.clone(), lane.mems.clone())),
        Role::Display => Some((lane.display.clone(), lane.mems.clone())),
        Role::Proxy | Role::Agent => None,
    })
}

/// A base64 SHA-256, as Chromium's `--ignore-certificate-errors-spki-list` takes it.
pub fn valid_spki(text: &str) -> bool {
    text.len() == 44 && text.ends_with('=') && text.bytes().all(|b| b.is_ascii_alphanumeric() || b"+/=".contains(&b))
}

/// A validated session request.
#[derive(Clone, Debug)]
pub struct Spec {
    pub session: SessionId,
    pub url: String,
    pub devtools: bool,
    pub width: u32,
    pub height: u32,
    pub timezone: String,
    pub languages: Vec<String>,
    /// The fixture certificate's SPKI hash, read at creation in the bench profile.
    pub bench_spki: Option<String>,
    /// The bench lane this session runs in (see [`Lane`]), if any.
    pub lane: Option<u8>,
    /// The upstream proxy the session's proxy dials through; it reaches the proxy on its standard input only.
    pub egress: Option<egress::Upstream>,
    /// Chrome in kiosk mode (the bench profile only, for the benchmark's like-for-like screen).
    pub kiosk: bool,
    /// The admin requires the walled garden for this session (the public demo).
    pub walled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    Display,
    Proxy,
    Browser,
    Agent,
}

impl Role {
    /// Every role, the agent last: it joins the browser's namespaces, so it is created and started after the rest.
    pub const ORDER: [Role; 4] = [Role::Display, Role::Proxy, Role::Browser, Role::Agent];

    pub fn name(self) -> &'static str {
        match self {
            Role::Display => "display",
            Role::Proxy => "proxy",
            Role::Browser => "browser",
            Role::Agent => "agent",
        }
    }
}

/// The session's one network: the proxy's way out. The browser has none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Net {
    Egress,
}

impl Net {
    pub const ORDER: [Net; 1] = [Net::Egress];

    fn role(self) -> &'static str {
        match self {
            Net::Egress => "egress",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Vol {
    Wayland,
    Agent,
    Bus,
    Proxy,
}

impl Vol {
    pub const ORDER: [Vol; 4] = [Vol::Wayland, Vol::Agent, Vol::Bus, Vol::Proxy];

    fn role(self) -> &'static str {
        match self {
            Vol::Wayland => "wayland",
            Vol::Agent => "agent",
            Vol::Bus => "bus",
            Vol::Proxy => "proxy",
        }
    }

    /// Owner, group, mode, and size of the tmpfs root.
    fn tmpfs(self) -> (u32, u32, &'static str, &'static str) {
        match self {
            Vol::Wayland => (UID_DISPLAY, GID_WAYLAND, "0750", "1m"),
            Vol::Agent => (UID_DISPLAY, GID_AGENT_SOCKETS, "0750", "1m"),
            Vol::Bus => (UID_BROWSER, GID_BUS, "0750", "8m"),
            Vol::Proxy => (UID_PROXY, GID_PROXY, "0750", "1m"),
        }
    }
}

fn labels(instance: &str, session: &SessionId, role: &str) -> HashMap<String, String> {
    HashMap::from([
        (LABEL_INSTANCE.to_owned(), instance.to_owned()),
        (LABEL_SESSION.to_owned(), session.to_string()),
        (LABEL_ROLE.to_owned(), role.to_owned()),
    ])
}

/// The prefix every resource of one session's name starts with.
pub fn session_prefix(instance: &str, session: &SessionId) -> String {
    format!("{instance}-{session}-")
}

pub fn container_name(instance: &str, session: &SessionId, role: Role) -> String {
    format!("{}{}", session_prefix(instance, session), role.name())
}

pub fn network_name(instance: &str, session: &SessionId, net: Net) -> String {
    format!("{}{}", session_prefix(instance, session), net.role())
}

pub fn volume_name(instance: &str, session: &SessionId, vol: Vol) -> String {
    format!("{}{}", session_prefix(instance, session), vol.role())
}

/// Every session bridge's host interface is named `er-<session>` (15 bytes, the kernel's limit), so one host rule,
/// `iptables -t raw -I PREROUTING 1 -i er-+ -m addrtype --dst-type LOCAL -j DROP`, keeps sessions from the addresses
/// the host holds, which the proxy's public-address policy admits for a public one: in the raw table, before Docker's
/// port redirects, so the ports Docker publishes there are covered too.
pub const BRIDGE_PREFIX: &str = "er-";
const BRIDGE_NAME: &str = "com.docker.network.bridge.name";

pub fn bridge_name(session: &SessionId) -> String {
    format!("{BRIDGE_PREFIX}{session}")
}

/// A session network; `internal` (the demo's walled garden) gives it no route off the host.
pub fn network_body(instance: &str, session: &SessionId, net: Net, internal: bool) -> NetworkCreateRequest {
    NetworkCreateRequest {
        name: network_name(instance, session, net),
        driver: Some("bridge".to_owned()),
        options: Some(HashMap::from([(BRIDGE_NAME.to_owned(), bridge_name(session))])),
        internal: Some(internal),
        attachable: Some(false),
        enable_ipv4: Some(true),
        enable_ipv6: Some(false),
        labels: Some(labels(instance, session, net.role())),
        ..Default::default()
    }
}

pub fn volume_body(instance: &str, session: &SessionId, vol: Vol) -> VolumeCreateRequest {
    let (uid, gid, mode, size) = vol.tmpfs();
    let options = format!("uid={uid},gid={gid},mode={mode},size={size},noexec,nosuid,nodev");
    VolumeCreateRequest {
        name: Some(volume_name(instance, session, vol)),
        driver: Some("local".to_owned()),
        driver_opts: Some(HashMap::from([
            ("type".to_owned(), "tmpfs".to_owned()),
            ("device".to_owned(), "tmpfs".to_owned()),
            ("o".to_owned(), options),
        ])),
        labels: Some(labels(instance, session, vol.role())),
        ..Default::default()
    }
}

fn volume(source: String, target: &str, read_only: bool, subpath: Option<String>) -> Mount {
    Mount {
        typ: Some(MountType::VOLUME),
        source: Some(source),
        target: Some(target.to_owned()),
        read_only: Some(read_only),
        // Never copy the image's directory (and its ownership) into the volume.
        volume_options: Some(MountVolumeOptions { no_copy: Some(true), subpath, ..Default::default() }),
        ..Default::default()
    }
}

fn tmpfs(entries: &[(&str, &str)]) -> HashMap<String, String> {
    entries.iter().map(|(path, extra)| ((*path).to_owned(), format!("{NOEXEC_TMPFS},{extra}"))).collect()
}

/// The uid that runs the agent: its own uid by default, Chromium's in DevTools mode (docs/agent-compatibility.md).
pub fn agent_uid(devtools: bool) -> u32 {
    if devtools { UID_BROWSER } else { UID_AGENT }
}

/// The user string of the agent container and of its one exec.
pub fn agent_user(devtools: bool) -> String {
    let uid = agent_uid(devtools);
    format!("{uid}:{uid}")
}

/// The period over which a container's CPU share is measured, in microseconds. Short: a container that spends its
/// quota early in a period (its threads busy on many cores at once) pauses until the next, and a 100 ms period would
/// pause its sound and input threads for most of one.
pub(crate) const CPU_PERIOD_US: i64 = 10_000;

/// A share of `cpus_milli` thousandths of a CPU, as CPU time per [`CPU_PERIOD_US`], in microseconds.
pub(crate) fn cpu_quota_us(cpus_milli: i64) -> i64 {
    cpus_milli * CPU_PERIOD_US / 1000
}

/// The display's CPU: two, so a recording encodes beside the live stream on a core of its own; one in a benchmark
/// lane, which measures it against a baseline at the same budget.
pub(crate) fn display_cpus_milli(spec: &Spec) -> i64 {
    if spec.lane.is_some() { 1000 } else { 2000 }
}

/// A container's limits, soft and hard, whatever the daemon's defaults. No core files, in every container: a host whose
/// core pattern hands crashes to a collector (systemd-coredump, apport) would otherwise write a crashed process's memory,
/// a page's cookies or what the person typed among it, to the host's disk. The display also gets room for the
/// descriptors it may hold at once (`MAX_OPEN_FDS` in display/src/limits.rs, which it sets as its soft limit at startup,
/// refusing to start where the hard limit is lower; a test holds this limit at or above it).
pub(crate) fn ulimits(role: Role) -> Vec<ResourcesUlimits> {
    const NOFILE: i64 = 32768;
    let limit =
        |name: &str, value| ResourcesUlimits { name: Some(name.to_owned()), soft: Some(value), hard: Some(value) };
    let mut limits = vec![limit("core", 0)];
    if role == Role::Display {
        limits.push(limit("nofile", NOFILE));
    }
    limits
}

/// The hardening every container shares.
fn hardened(cpus_milli: i64, memory_mib: i64, pids: i64, network_mode: String) -> HostConfig {
    HostConfig {
        cpu_period: Some(CPU_PERIOD_US),
        cpu_quota: Some(cpu_quota_us(cpus_milli)),
        memory: Some(memory_mib * MIB),
        memory_swap: Some(memory_mib * MIB),
        pids_limit: Some(pids),
        init: Some(true),
        readonly_rootfs: Some(true),
        privileged: Some(false),
        cap_drop: Some(vec!["ALL".to_owned()]),
        cap_add: None,
        security_opt: Some(vec!["no-new-privileges:true".to_owned()]),
        network_mode: Some(network_mode),
        ipc_mode: Some("private".to_owned()),
        cgroupns_mode: Some(HostConfigCgroupnsModeEnum::PRIVATE),
        publish_all_ports: Some(false),
        log_config: Some(HostConfigLogConfig {
            typ: Some("local".to_owned()),
            config: Some(HashMap::from([
                ("max-size".to_owned(), "10m".to_owned()),
                ("max-file".to_owned(), "3".to_owned()),
            ])),
        }),
        ..Default::default()
    }
}

fn groups(ids: &[u32]) -> Option<Vec<String>> {
    Some(ids.iter().map(u32::to_string).collect())
}

fn env(pairs: &[(&str, String)]) -> Option<Vec<String>> {
    Some(pairs.iter().map(|(k, v)| format!("{k}={v}")).collect())
}

/// The create body of one role. `browser_id` is the browser container's id, required for the agent.
pub fn container_body(
    fixed: &Fixed,
    spec: &Spec,
    role: Role,
    browser_id: Option<&str>,
) -> Result<ContainerCreateBody, String> {
    let s = &spec.session;
    let mut body = ContainerCreateBody {
        labels: Some(labels(&fixed.instance, s, role.name())),
        network_disabled: None,
        attach_stdin: Some(false),
        attach_stdout: Some(false),
        attach_stderr: Some(false),
        open_stdin: Some(false),
        tty: Some(false),
        ..Default::default()
    };
    let egress = network_name(&fixed.instance, s, Net::Egress);
    match role {
        Role::Display => {
            let agent_uid = agent_uid(spec.devtools);
            body.image = Some(fixed.images.display.clone());
            body.user = Some(format!("{UID_DISPLAY}:{UID_DISPLAY}"));
            body.env = env(&[
                ("DISPLAY_OUTPUT", format!("{}x{}", spec.width, spec.height)),
                ("DISPLAY_BROWSER_SOCKET", format!("{DISPLAY_BROWSER_DIR}/wayland-0")),
                ("DISPLAY_BROWSER_UID", UID_BROWSER.to_string()),
                ("DISPLAY_AGENT_SOCKET", format!("{DISPLAY_AGENT_DIR}/wayland-0")),
                ("DISPLAY_INJECT_SOCKET", format!("{DISPLAY_AGENT_DIR}/cua-inject")),
                ("DISPLAY_AGENT_UID", agent_uid.to_string()),
                ("DISPLAY_STREAM_SOCKET", format!("{DISPLAY_STREAM_DIR}/display.sock")),
                ("DISPLAY_ADMIN_UID", UID_ADMIN.to_string()),
                ("DISPLAY_AUDIO_FIFO", format!("{DISPLAY_BROWSER_DIR}/audio")),
            ]);
            let mut host = hardened(display_cpus_milli(spec), 512, 256, "none".to_owned());
            host.group_add = groups(&[GID_WAYLAND, GID_AGENT_SOCKETS, GID_STREAMS]);
            host.tmpfs = Some(tmpfs(&[("/tmp", "size=64m")]));
            host.mounts = Some(vec![
                volume(volume_name(&fixed.instance, s, Vol::Wayland), DISPLAY_BROWSER_DIR, false, None),
                volume(volume_name(&fixed.instance, s, Vol::Agent), DISPLAY_AGENT_DIR, false, None),
                volume(fixed.streams_volume.clone(), DISPLAY_STREAM_DIR, false, Some(s.to_string())),
            ]);
            body.host_config = Some(host);
        }
        Role::Proxy => {
            body.image = Some(fixed.images.proxy.clone());
            body.user = Some(format!("{UID_PROXY}:{UID_PROXY}"));
            let mut vars = vec![
                ("PROXY_LISTEN_UNIX", format!("{PROXY_DIR}/proxy.sock")),
                ("PROXY_CLIENT_UID", UID_BROWSER.to_string()),
            ];
            let mut endpoints = HashMap::from([(egress.clone(), EndpointSettings::default())]);
            if spec.egress.is_some() {
                // The upstream itself comes on standard input (ops.rs), never in the environment.
                vars.push(("PROXY_UPSTREAM_STDIN", "1".to_owned()));
                body.open_stdin = Some(true);
                body.stdin_once = Some(true);
                body.attach_stdin = Some(true);
            }
            if let Some(bench) = &fixed.bench {
                vars.push(("PROXY_BENCH_FIXTURE", bench.fixture.clone()));
                if bench.fixture_only {
                    vars.push(("PROXY_FIXTURE_ONLY", "1".to_owned()));
                }
                endpoints.insert(bench.network.clone(), EndpointSettings::default());
            }
            body.env = env(&vars);
            let mut host = hardened(500, 128, 256, egress);
            host.group_add = groups(&[GID_PROXY]);
            host.sysctls = Some(HashMap::from([("net.ipv4.ip_forward".to_owned(), "0".to_owned())]));
            host.tmpfs = Some(tmpfs(&[("/tmp", "size=8m")]));
            host.mounts = Some(vec![volume(volume_name(&fixed.instance, s, Vol::Proxy), PROXY_DIR, false, None)]);
            body.host_config = Some(host);
            body.networking_config = Some(NetworkingConfig { endpoints_config: Some(endpoints) });
        }
        Role::Browser => {
            body.image = Some(fixed.images.browser.clone());
            body.user = Some(format!("{UID_BROWSER}:{UID_BROWSER}"));
            body.env = env(&[
                ("BROWSER_URL", spec.url.clone()),
                ("BROWSER_SIZE", format!("{}x{}", spec.width, spec.height)),
                ("BROWSER_TIMEZONE", spec.timezone.clone()),
                ("BROWSER_LANGUAGES", spec.languages.join(",")),
                ("BROWSER_DEVTOOLS", if spec.devtools { "1" } else { "0" }.to_owned()),
                ("BROWSER_PROXY_SOCKET", format!("{BROWSER_PROXY_DIR}/proxy.sock")),
            ]);
            if let (Some(spki), Some(vars)) = (&spec.bench_spki, body.env.as_mut()) {
                vars.push(format!("BROWSER_BENCH_SPKI={spki}"));
            }
            if spec.kiosk {
                if fixed.bench.is_none() {
                    return Err("refused: kiosk sessions are the bench profile's alone".into());
                }
                body.env.get_or_insert_default().push("BROWSER_KIOSK=1".to_owned());
            }
            let mut host = hardened(fixed.browser.cpus_milli, fixed.browser.memory_mib, 2048, "none".to_owned());
            host.group_add = groups(&[GID_WAYLAND, GID_BUS, GID_PROXY]);
            host.shm_size = Some(512 * MIB);
            if let Some(opts) = host.security_opt.as_mut() {
                opts.push(format!("seccomp={}", fixed.browser_seccomp));
            }
            let private = format!("size=256m,uid={UID_BROWSER},gid={UID_BROWSER},mode=0700");
            // The profile reads as an ordinary disk, so pages see the usual storage quota; Chrome keeps each site's
            // storage small (browser/launch) and the browser's memory limit bounds the whole.
            let profile = format!("size=16g,uid={UID_BROWSER},gid={UID_BROWSER},mode=0700");
            // Files the person uploads (the launcher stores them; the browser's memory limit still applies).
            let uploads = format!("size=1g,uid={UID_BROWSER},gid={UID_BROWSER},mode=0700");
            host.tmpfs = Some(tmpfs(&[
                ("/tmp", "size=256m"),
                ("/home/browser", &profile),
                ("/downloads", &private),
                ("/uploads", &uploads),
            ]));
            host.mounts = Some(vec![
                volume(volume_name(&fixed.instance, s, Vol::Wayland), BROWSER_WAYLAND_DIR, true, None),
                volume(volume_name(&fixed.instance, s, Vol::Bus), BUS_DIR, false, None),
                volume(volume_name(&fixed.instance, s, Vol::Proxy), BROWSER_PROXY_DIR, true, None),
            ]);
            body.host_config = Some(host);
        }
        Role::Agent => {
            let browser = browser_id.ok_or("the agent template needs the browser container")?;
            if browser.is_empty() || !browser.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err("the browser container id must be hex".into());
            }
            body.image = Some(fixed.images.agent.clone());
            body.user = Some(agent_user(spec.devtools));
            body.env = env(&[
                ("HOME", "/tmp".to_owned()),
                ("WAYLAND_DISPLAY", format!("{AGENT_DIR}/wayland-0")),
                ("CUA_INJECT_SOCKET", format!("{AGENT_DIR}/cua-inject")),
                ("CUA_DRIVER_RS_ENABLE_WAYLAND", "1".to_owned()),
                ("DBUS_SESSION_BUS_ADDRESS", format!("unix:path={BUS_DIR}/session")),
                ("AGENT_DEVTOOLS", if spec.devtools { "1" } else { "0" }.to_owned()),
            ]);
            let network = if spec.devtools { format!("container:{browser}") } else { "none".to_owned() };
            let mut host = hardened(500, 512, 512, network);
            // In the browser's PID namespace Docker would drop `init` anyway; were it applied, its docker-init would be
            // a dumpable process in the agent's mount namespace.
            host.init = None;
            host.pid_mode = Some(format!("container:{browser}"));
            host.group_add = groups(&[GID_BUS, GID_AGENT_SOCKETS]);
            host.tmpfs = Some(tmpfs(&[("/tmp", "size=64m")]));
            host.mounts = Some(vec![
                volume(volume_name(&fixed.instance, s, Vol::Agent), AGENT_DIR, true, None),
                volume(volume_name(&fixed.instance, s, Vol::Bus), BUS_DIR, true, None),
            ]);
            body.host_config = Some(host);
        }
    }
    if let Some(host) = body.host_config.as_mut() {
        host.ulimits = Some(ulimits(role));
    }
    if let (Some((cpus, mems)), Some(host)) = (placement(fixed, spec, role)?, body.host_config.as_mut()) {
        host.cpuset_cpus = Some(cpus);
        host.cpuset_mems = Some(mems);
    }
    check_container(fixed, spec, role, browser_id, &body)?;
    Ok(body)
}

// The display's own bounds, read from its source: it is a binary crate of its own.
#[cfg(test)]
#[allow(dead_code)]
#[path = "../../display/src/limits.rs"]
mod display_limits;

#[cfg(test)]
mod tests {
    use super::*;

    const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const BROWSER_ID: &str = "b0b0b0b0b0b0b0b0";

    fn fixed() -> Fixed {
        let image = |name: &str| format!("example.test/{name}@sha256:{DIGEST}");
        Fixed {
            instance: "easirun".into(),
            images: Images::new(&image("display"), &image("browser"), &image("agent"), &image("proxy")).unwrap(),
            browser: BrowserLimits::default(),
            browser_seccomp: r#"{"defaultAction":"SCMP_ACT_ERRNO"}"#.into(),
            streams_volume: "display_streams".into(),
            bench: None,
        }
    }

    fn spec(devtools: bool) -> Spec {
        Spec {
            session: "0123456789ab".parse().unwrap(),
            url: "https://example.com/".into(),
            devtools,
            width: 1280,
            height: 800,
            timezone: "America/Sao_Paulo".into(),
            languages: vec!["pt-BR".into(), "pt".into(), "en-US".into(), "en".into()],
            bench_spki: None,
            lane: None,
            egress: None,
            kiosk: false,
            walled: false,
        }
    }

    fn bodies(devtools: bool) -> Vec<(Role, ContainerCreateBody)> {
        Role::ORDER
            .iter()
            .map(|&role| (role, container_body(&fixed(), &spec(devtools), role, Some(BROWSER_ID)).unwrap()))
            .collect()
    }

    fn host(body: &ContainerCreateBody) -> &HostConfig {
        body.host_config.as_ref().unwrap()
    }

    fn env_of(body: &ContainerCreateBody, key: &str) -> Option<String> {
        body.env.iter().flatten().find_map(|e| e.strip_prefix(&format!("{key}=")).map(str::to_owned))
    }

    #[test]
    fn the_displays_descriptor_limit_holds_what_it_may_open() {
        let [_, nofile] = &ulimits(Role::Display)[..] else { panic!("core and nofile") };
        let open = i64::try_from(display_limits::MAX_OPEN_FDS).unwrap();
        assert!(nofile.soft == nofile.hard && nofile.soft >= Some(open), "{nofile:?} < MAX_OPEN_FDS {open}");
    }

    #[test]
    fn every_container_is_hardened() {
        for devtools in [false, true] {
            for (role, body) in bodies(devtools) {
                let host = host(&body);
                assert_eq!(host.privileged, Some(false), "{role:?}");
                assert_eq!(host.cap_drop.as_deref(), Some(&["ALL".to_owned()][..]), "{role:?}");
                assert!(host.cap_add.is_none(), "{role:?}");
                assert_eq!(host.readonly_rootfs, Some(true), "{role:?}");
                assert_eq!(host.init, (role != Role::Agent).then_some(true), "{role:?}");
                assert!(host.security_opt.iter().flatten().any(|o| o == "no-new-privileges:true"), "{role:?}");
                assert!(host.binds.is_none() && host.volumes_from.is_none(), "{role:?}");
                assert!(host.devices.is_none() && host.device_requests.is_none(), "{role:?}");
                assert!(host.port_bindings.is_none(), "{role:?}");
                assert_eq!(host.ipc_mode.as_deref(), Some("private"), "{role:?}");
                for mount in host.mounts.iter().flatten() {
                    assert_eq!(mount.typ, Some(MountType::VOLUME), "{role:?}");
                    assert!(mount.bind_options.is_none(), "{role:?}");
                }
                assert!(body.cmd.is_none() && body.entrypoint.is_none(), "{role:?}");
                let labels = body.labels.as_ref().unwrap();
                assert_eq!(labels[LABEL_SESSION], "0123456789ab");
                assert_eq!(labels[LABEL_ROLE], role.name());
            }
        }
    }

    #[test]
    fn exact_users_and_groups() {
        let expect = |devtools, role: Role| -> (&'static str, Vec<&'static str>) {
            match (role, devtools) {
                (Role::Display, _) => ("10002:10002", vec!["10011", "10013", "10014"]),
                (Role::Proxy, _) => ("10003:10003", vec!["10016"]),
                (Role::Browser, _) => ("10001:10001", vec!["10011", "10012", "10016"]),
                (Role::Agent, false) => ("10005:10005", vec!["10012", "10013"]),
                (Role::Agent, true) => ("10001:10001", vec!["10012", "10013"]),
            }
        };
        for devtools in [false, true] {
            for (role, body) in bodies(devtools) {
                let (user, groups) = expect(devtools, role);
                assert_eq!(body.user.as_deref(), Some(user), "{role:?}");
                let added: Vec<&str> = host(&body).group_add.iter().flatten().map(String::as_str).collect();
                assert_eq!(added, groups, "{role:?}");
            }
        }
    }

    #[test]
    fn browser_limits_parse_within_bounds_and_the_guard_holds_them() {
        assert_eq!(BrowserLimits::parse(None, None), Ok(BrowserLimits::default()));
        assert_eq!(
            BrowserLimits::parse(Some("3"), Some("4096")),
            Ok(BrowserLimits { cpus_milli: 3000, memory_mib: 4096 })
        );
        assert_eq!(BrowserLimits::parse(Some(" 0.75 "), None).map(|l| l.cpus_milli), Ok(750));
        for (cpus, memory) in [(Some("0.4"), None), (Some("17"), None), (Some("NaN"), None), (Some("two"), None)] {
            assert!(BrowserLimits::parse(cpus, memory).is_err(), "{cpus:?}");
        }
        assert!(BrowserLimits::parse(None, Some("512")).is_err());
        assert!(BrowserLimits::parse(None, Some("1.5")).is_err());
        // A browser body built with other limits is refused by a controller configured with the defaults.
        let mut wider = fixed();
        wider.browser = BrowserLimits { cpus_milli: 4000, memory_mib: 2048 };
        let body = container_body(&wider, &spec(false), Role::Browser, Some(BROWSER_ID)).unwrap();
        assert!(check_container(&fixed(), &spec(false), Role::Browser, Some(BROWSER_ID), &body).is_err());
    }

    #[test]
    fn resource_limits() {
        for (role, body) in bodies(false) {
            let host = host(&body);
            // CPU time per 10 ms period, in microseconds.
            let (quota, memory) = match role {
                Role::Display => (20_000, 512 * MIB),
                Role::Proxy => (5_000, 128 * MIB),
                Role::Browser => (20_000, 2048 * MIB),
                Role::Agent => (5_000, 512 * MIB),
            };
            assert_eq!(
                (host.nano_cpus, host.cpu_period, host.cpu_quota),
                (None, Some(10_000), Some(quota)),
                "{role:?}"
            );
            assert_eq!(host.memory, Some(memory), "{role:?}");
            assert_eq!(host.memory_swap, Some(memory), "{role:?}");
            assert!(host.pids_limit.unwrap() > 0, "{role:?}");
            let shm = if role == Role::Browser { Some(512 * MIB) } else { None };
            assert_eq!(host.shm_size, shm, "{role:?}");
        }
    }

    #[test]
    fn network_modes() {
        let egress = "easirun-0123456789ab-egress";
        for devtools in [false, true] {
            for (role, body) in bodies(devtools) {
                let host = host(&body);
                let endpoints: Vec<&str> = body
                    .networking_config
                    .iter()
                    .flat_map(|n| n.endpoints_config.iter().flat_map(|e| e.keys()))
                    .map(String::as_str)
                    .collect();
                match role {
                    Role::Display => {
                        assert_eq!(host.network_mode.as_deref(), Some("none"));
                        assert!(endpoints.is_empty());
                    }
                    Role::Proxy => {
                        assert_eq!(host.network_mode.as_deref(), Some(egress));
                        assert_eq!(endpoints, [egress]);
                        assert_eq!(host.sysctls.as_ref().unwrap()["net.ipv4.ip_forward"], "0");
                    }
                    Role::Browser => {
                        // No network at all: the proxy is reached through its socket.
                        assert_eq!(host.network_mode.as_deref(), Some("none"));
                        assert!(endpoints.is_empty());
                    }
                    Role::Agent => {
                        let net = if devtools { format!("container:{BROWSER_ID}") } else { "none".into() };
                        assert_eq!(host.network_mode.as_deref(), Some(net.as_str()));
                        assert_eq!(host.pid_mode.as_deref(), Some(format!("container:{BROWSER_ID}").as_str()));
                        assert!(endpoints.is_empty());
                    }
                }
                if role != Role::Agent {
                    assert!(host.pid_mode.is_none(), "{role:?}");
                }
            }
        }
    }

    #[test]
    fn mounts_and_subpath() {
        let all = bodies(false);
        let mounts = |role: Role| -> Vec<(String, String, bool, Option<String>)> {
            let body = &all.iter().find(|(r, _)| *r == role).unwrap().1;
            host(body)
                .mounts
                .iter()
                .flatten()
                .map(|m| {
                    let options = m.volume_options.as_ref().unwrap();
                    assert_eq!(options.no_copy, Some(true));
                    (
                        m.source.clone().unwrap(),
                        m.target.clone().unwrap(),
                        m.read_only.unwrap(),
                        options.subpath.clone(),
                    )
                })
                .collect()
        };
        let m =
            |s: &str, t: &str, ro: bool, sub: Option<&str>| (s.to_owned(), t.to_owned(), ro, sub.map(str::to_owned));
        assert_eq!(
            mounts(Role::Display),
            [
                m("easirun-0123456789ab-wayland", "/run/display/browser", false, None),
                m("easirun-0123456789ab-agent", "/run/display/agent", false, None),
                m("display_streams", "/run/display/stream", false, Some("0123456789ab")),
            ]
        );
        assert_eq!(
            mounts(Role::Browser),
            [
                m("easirun-0123456789ab-wayland", "/run/browser/wayland", true, None),
                m("easirun-0123456789ab-bus", "/run/browser/bus", false, None),
                m("easirun-0123456789ab-proxy", "/run/browser/proxy", true, None),
            ]
        );
        assert_eq!(mounts(Role::Proxy), [m("easirun-0123456789ab-proxy", "/run/proxy", false, None)]);
        assert_eq!(
            mounts(Role::Agent),
            [
                m("easirun-0123456789ab-agent", "/run/agent", true, None),
                m("easirun-0123456789ab-bus", "/run/browser/bus", true, None)
            ]
        );
        for (role, body) in &all {
            for (path, options) in host(body).tmpfs.iter().flatten() {
                assert!(
                    options.contains("noexec") && options.contains("nosuid") && options.contains("nodev"),
                    "{role:?} {path}"
                );
            }
        }
        let browser_tmpfs = host(&all[2].1).tmpfs.clone().unwrap();
        let mut paths: Vec<_> = browser_tmpfs.keys().map(String::as_str).collect();
        paths.sort();
        assert_eq!(paths, ["/downloads", "/home/browser", "/tmp", "/uploads"]);
    }

    #[test]
    fn environment() {
        for devtools in [false, true] {
            let all = bodies(devtools);
            let display = &all[0].1;
            assert_eq!(env_of(display, "DISPLAY_BROWSER_SOCKET").unwrap(), "/run/display/browser/wayland-0");
            assert_eq!(env_of(display, "DISPLAY_AGENT_SOCKET").unwrap(), "/run/display/agent/wayland-0");
            assert_eq!(env_of(display, "DISPLAY_INJECT_SOCKET").unwrap(), "/run/display/agent/cua-inject");
            assert_eq!(env_of(display, "DISPLAY_STREAM_SOCKET").unwrap(), "/run/display/stream/display.sock");
            // The browser's sound arrives in the Wayland volume, which the browser mounts read-only.
            assert_eq!(env_of(display, "DISPLAY_AUDIO_FIFO").unwrap(), "/run/display/browser/audio");
            assert_eq!(env_of(display, "DISPLAY_BROWSER_UID").unwrap(), "10001");
            assert_eq!(env_of(display, "DISPLAY_AGENT_UID").unwrap(), if devtools { "10001" } else { "10005" });
            assert_eq!(env_of(display, "DISPLAY_ADMIN_UID").unwrap(), "10004");
            assert_eq!(env_of(display, "DISPLAY_OUTPUT").unwrap(), "1280x800");
            let browser = &all[2].1;
            assert_eq!(env_of(browser, "BROWSER_URL").unwrap(), "https://example.com/");
            assert_eq!(env_of(browser, "BROWSER_SIZE").unwrap(), "1280x800");
            assert_eq!(env_of(browser, "BROWSER_TIMEZONE").unwrap(), "America/Sao_Paulo");
            assert_eq!(env_of(browser, "BROWSER_LANGUAGES").unwrap(), "pt-BR,pt,en-US,en");
            assert_eq!(env_of(browser, "BROWSER_DEVTOOLS").unwrap(), if devtools { "1" } else { "0" });
            assert_eq!(env_of(browser, "BROWSER_PROXY_SOCKET").unwrap(), "/run/browser/proxy/proxy.sock");
            let proxy = &all[1].1;
            assert_eq!(env_of(proxy, "PROXY_LISTEN_UNIX").unwrap(), "/run/proxy/proxy.sock");
            assert_eq!(env_of(proxy, "PROXY_CLIENT_UID").unwrap(), "10001");
            let agent = &all[3].1;
            assert_eq!(env_of(agent, "WAYLAND_DISPLAY").unwrap(), "/run/agent/wayland-0");
            assert_eq!(env_of(agent, "CUA_INJECT_SOCKET").unwrap(), "/run/agent/cua-inject");
            assert_eq!(env_of(agent, "AGENT_DEVTOOLS").unwrap(), if devtools { "1" } else { "0" });
        }
    }

    #[test]
    fn kiosk_mode_is_the_bench_profiles_alone() {
        let mut fixed = fixed();
        let mut spec = spec(false);
        spec.kiosk = true;
        assert!(container_body(&fixed, &spec, Role::Browser, None).is_err());
        fixed.bench = Some(Bench {
            network: "easi-run-bench".into(),
            fixture: "fixture.bench=172.31.250.10".into(),
            spki_file: "/run/bench/spki".into(),
            lanes: Vec::new(),
            fixture_only: false,
        });
        let body = container_body(&fixed, &spec, Role::Browser, None).unwrap();
        assert_eq!(env_of(&body, "BROWSER_KIOSK").as_deref(), Some("1"));
        spec.kiosk = false;
        let body = container_body(&fixed, &spec, Role::Browser, None).unwrap();
        assert_eq!(env_of(&body, "BROWSER_KIOSK"), None);
    }

    #[test]
    fn the_walled_garden_reaches_the_fixture_alone() {
        let mut fixed = fixed();
        let bench = |fixture_only| Bench {
            network: "easi-run-demo-play".into(),
            fixture: "home.play=172.31.250.10".into(),
            spki_file: "/run/bench/spki".into(),
            lanes: Vec::new(),
            fixture_only,
        };
        fixed.bench = Some(bench(true));
        let body = container_body(&fixed, &spec(false), Role::Proxy, None).unwrap();
        assert_eq!(env_of(&body, "PROXY_FIXTURE_ONLY").as_deref(), Some("1"));
        assert_eq!(env_of(&body, "PROXY_BENCH_FIXTURE").as_deref(), Some("home.play=172.31.250.10"));
        fixed.bench = Some(bench(false));
        let body = container_body(&fixed, &spec(false), Role::Proxy, None).unwrap();
        assert_eq!(env_of(&body, "PROXY_FIXTURE_ONLY"), None);
        let s = spec(false).session;
        assert_eq!(network_body("easirun-demo", &s, Net::Egress, true).internal, Some(true));
        assert_eq!(network_body("easirun", &s, Net::Egress, false).internal, Some(false));
    }

    #[test]
    fn a_bench_lane_places_the_browser_and_the_display_and_nothing_else() {
        let mut fixed = fixed();
        let lane = Lane::parse("4,52/5,53/0").unwrap();
        fixed.bench = Some(Bench {
            network: "easi-run-bench".into(),
            fixture: "fixture.bench=172.31.250.10".into(),
            spki_file: "/run/bench/spki".into(),
            lanes: vec![lane],
            fixture_only: false,
        });
        let mut spec = spec(false);
        spec.lane = Some(0);
        let placed = |role| {
            let host = container_body(&fixed, &spec, role, Some("abc123")).unwrap().host_config.unwrap();
            (host.cpuset_cpus, host.cpuset_mems, host.cpu_period, host.cpu_quota)
        };
        assert_eq!(placed(Role::Browser), (Some("4,52".into()), Some("0".into()), Some(10_000), Some(20_000)));
        assert_eq!(
            placed(Role::Display),
            (Some("5,53".into()), Some("0".into()), Some(10_000), Some(10_000)),
            "one CPU"
        );
        assert_eq!(placed(Role::Proxy).0, None);
        assert_eq!(placed(Role::Agent).0, None);
        // An undeclared lane, or a lane outside the bench profile, is refused.
        spec.lane = Some(1);
        assert!(container_body(&fixed, &spec, Role::Browser, None).is_err());
        spec.lane = Some(0);
        assert!(container_body(&self::fixed(), &spec, Role::Browser, None).is_err());
        // The guard refuses a placement the lane does not name, and any placement without a lane.
        let mut body = container_body(&fixed, &spec, Role::Browser, None).unwrap();
        body.host_config.as_mut().unwrap().cpuset_cpus = Some("0-95".into());
        assert!(check_container(&fixed, &spec, Role::Browser, None, &body).is_err());
        spec.lane = None;
        let mut body = container_body(&fixed, &spec, Role::Display, None).unwrap();
        body.host_config.as_mut().unwrap().cpuset_mems = Some("0".into());
        assert!(check_container(&fixed, &spec, Role::Display, None, &body).is_err());
        for bad in ["", "1/2", "1/2/3/4", "a/2/0", "1;2/3/0", "1,,2/3/0", "-1/2/0", "12345/1/0"] {
            assert!(Lane::parse(bad).is_err(), "{bad:?}");
        }
        assert!(Lane::parse("0-3,48-51/4/0,1").is_ok());
    }

    #[test]
    fn the_bench_profile_adds_only_the_fixture_exception() {
        let mut fixed = fixed();
        fixed.bench = Some(Bench {
            network: "easi-run-bench".into(),
            fixture: "fixture.bench=172.31.250.10".into(),
            spki_file: "/run/bench/spki".into(),
            lanes: vec![],
            fixture_only: false,
        });
        let mut spec = spec(false);
        spec.bench_spki = Some("47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU=".into());
        let proxy = container_body(&fixed, &spec, Role::Proxy, None).unwrap();
        assert_eq!(env_of(&proxy, "PROXY_BENCH_FIXTURE").unwrap(), "fixture.bench=172.31.250.10");
        let networks = proxy.networking_config.as_ref().unwrap().endpoints_config.as_ref().unwrap();
        assert!(networks.contains_key("easi-run-bench") && networks.len() == 2);
        check_container(&fixed, &spec, Role::Proxy, None, &proxy).unwrap();
        let browser = container_body(&fixed, &spec, Role::Browser, None).unwrap();
        assert_eq!(env_of(&browser, "BROWSER_BENCH_SPKI").unwrap(), spec.bench_spki.clone().unwrap());
        assert!(browser.networking_config.is_none(), "the browser joins no network, bench or not");
        // Without the bench profile, the same network is refused.
        assert!(check_container(&self::fixed(), &spec, Role::Proxy, None, &proxy).is_err());
        assert!(valid_spki("47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU="));
        assert!(!valid_spki("not a hash"));
    }

    #[test]
    fn only_the_browser_gets_the_seccomp_profile() {
        for (role, body) in bodies(false) {
            let seccomp: Vec<_> =
                host(&body).security_opt.iter().flatten().filter(|o| o.starts_with("seccomp")).collect();
            if role == Role::Browser {
                assert_eq!(seccomp, [&format!("seccomp={}", fixed().browser_seccomp)]);
            } else {
                assert!(seccomp.is_empty(), "{role:?}");
            }
        }
    }

    #[test]
    fn networks_and_volumes() {
        let s: SessionId = "0123456789ab".parse().unwrap();
        let egress = network_body("easirun", &s, Net::Egress, false);
        assert_eq!(egress.name, "easirun-0123456789ab-egress");
        assert_eq!(egress.internal, Some(false));
        assert_eq!(egress.enable_ipv6, Some(false));
        // The one option: the host interface's name, within the kernel's 15 bytes, under the documented prefix.
        let options = HashMap::from([("com.docker.network.bridge.name".to_owned(), "er-0123456789ab".to_owned())]);
        assert_eq!(egress.options, Some(options));
        assert!(bridge_name(&s).len() <= 15);
        for net in Net::ORDER {
            assert_eq!(network_body("easirun", &s, net, false).labels.unwrap()[LABEL_SESSION], "0123456789ab");
        }
        let expect = [
            (Vol::Wayland, "uid=10002,gid=10011,mode=0750"),
            (Vol::Agent, "uid=10002,gid=10013,mode=0750"),
            (Vol::Bus, "uid=10001,gid=10012,mode=0750"),
            (Vol::Proxy, "uid=10003,gid=10016,mode=0750"),
        ];
        for (vol, owner) in expect {
            let body = volume_body("easirun", &s, vol);
            let opts = body.driver_opts.unwrap();
            assert_eq!(opts["type"], "tmpfs");
            assert!(opts["o"].starts_with(owner), "{vol:?}");
            assert!(opts["o"].ends_with(",noexec,nosuid,nodev"), "{vol:?}");
            assert_eq!(body.labels.unwrap()[LABEL_SESSION], "0123456789ab");
        }
    }

    #[test]
    fn two_instances_never_share_a_name_or_a_label() {
        let s = spec(false).session;
        assert_ne!(network_name("easirun", &s, Net::Egress), network_name("demo", &s, Net::Egress));
        assert_eq!(network_body("demo", &s, Net::Egress, false).labels.unwrap()[LABEL_INSTANCE], "demo");
        for vol in [Vol::Wayland, Vol::Agent, Vol::Bus, Vol::Proxy] {
            assert!(volume_name("demo", &s, vol).starts_with("demo-0123456789ab-"));
            assert_eq!(volume_body("demo", &s, vol).labels.unwrap()[LABEL_INSTANCE], "demo");
        }
        for role in Role::ORDER {
            assert_ne!(container_name("easirun", &s, role), container_name("demo", &s, role));
        }
        let mut demo = fixed();
        demo.instance = "demo".into();
        for role in Role::ORDER {
            let body = container_body(&demo, &spec(false), role, Some(BROWSER_ID)).unwrap();
            assert_eq!(body.labels.as_ref().unwrap()[LABEL_INSTANCE], "demo", "{role:?}");
            assert!(check_container(&fixed(), &spec(false), role, Some(BROWSER_ID), &body).is_err(), "{role:?}");
        }
    }

    #[test]
    fn the_guard_refuses_anything_outside_the_templates() {
        let fixed = fixed();
        let spec = spec(false);
        let check = |role, body: &ContainerCreateBody| check_container(&fixed, &spec, role, Some(BROWSER_ID), body);
        let base = |role| container_body(&fixed, &spec, role, Some(BROWSER_ID)).unwrap();
        let mutated = |role, edit: &dyn Fn(&mut ContainerCreateBody)| {
            let mut body = base(role);
            edit(&mut body);
            check(role, &body)
        };
        fn h(b: &mut ContainerCreateBody) -> &mut HostConfig {
            b.host_config.as_mut().unwrap()
        }
        assert!(mutated(Role::Browser, &|b| h(b).binds = Some(vec!["/:/host".into()])).is_err());
        assert!(
            mutated(Role::Browser, &|b| h(b).mounts.as_mut().unwrap().push(Mount {
                typ: Some(MountType::BIND),
                source: Some("/".into()),
                target: Some("/host".into()),
                ..Default::default()
            }))
            .is_err()
        );
        assert!(
            mutated(Role::Display, &|b| {
                b.labels.as_mut().unwrap().insert(LABEL_INSTANCE.into(), "other".into());
            })
            .is_err()
        );
        assert!(
            mutated(Role::Display, &|b| {
                b.labels.as_mut().unwrap().remove(LABEL_INSTANCE);
            })
            .is_err()
        );
        // Another installation's volume, even for the same session id.
        assert!(
            mutated(Role::Browser, &|b| h(b).mounts.as_mut().unwrap()[0].source =
                Some("other-0123456789ab-wayland".into()))
            .is_err()
        );
        assert!(mutated(Role::Display, &|b| h(b).privileged = Some(true)).is_err());
        assert!(mutated(Role::Display, &|b| h(b).cap_add = Some(vec!["SYS_ADMIN".into()])).is_err());
        assert!(mutated(Role::Display, &|b| h(b).cap_drop = Some(vec![])).is_err());
        assert!(mutated(Role::Display, &|b| h(b).readonly_rootfs = Some(false)).is_err());
        assert!(mutated(Role::Display, &|b| h(b).security_opt = Some(vec![])).is_err());
        assert!(
            mutated(Role::Display, &|b| h(b).security_opt.as_mut().unwrap().push("seccomp=unconfined".into())).is_err()
        );
        assert!(
            mutated(Role::Display, &|b| h(b).security_opt.as_mut().unwrap().push("apparmor=unconfined".into()))
                .is_err()
        );
        assert!(mutated(Role::Proxy, &|b| h(b).network_mode = Some("host".into())).is_err());
        assert!(mutated(Role::Display, &|b| h(b).pid_mode = Some("host".into())).is_err());
        assert!(mutated(Role::Display, &|b| h(b).ipc_mode = Some("host".into())).is_err());
        assert!(mutated(Role::Display, &|b| h(b).userns_mode = Some("host".into())).is_err());
        assert!(mutated(Role::Display, &|b| h(b).uts_mode = Some("host".into())).is_err());
        assert!(mutated(Role::Agent, &|b| h(b).pid_mode = Some("container:ffff".into())).is_err());
        assert!(
            mutated(Role::Display, &|b| h(b).devices = Some(vec![bollard::models::DeviceMapping {
                path_on_host: Some("/dev/dri/renderD128".into()),
                ..Default::default()
            }]))
            .is_err()
        );
        assert!(mutated(Role::Display, &|b| b.user = Some("0:0".into())).is_err());
        assert!(mutated(Role::Display, &|b| b.image = Some("busybox:latest".into())).is_err());
        assert!(mutated(Role::Display, &|b| b.cmd = Some(vec!["sh".into()])).is_err());
        assert!(mutated(Role::Display, &|b| h(b).memory = None).is_err());
        assert!(mutated(Role::Display, &|b| h(b).mounts.as_mut().unwrap()[0].source = Some("other".into())).is_err());
        assert!(
            mutated(Role::Display, &|b| h(b).mounts.as_mut().unwrap()[2].volume_options.as_mut().unwrap().subpath =
                Some("ffffffffffff".into()))
            .is_err()
        );
        assert!(mutated(Role::Display, &|b| h(b).network_mode = Some("bridge".into())).is_err());
        assert!(
            mutated(Role::Browser, &|b| h(b).security_opt.as_mut().unwrap().retain(|o| !o.starts_with("seccomp")))
                .is_err()
        );
        assert!(container_body(&fixed, &spec, Role::Agent, None).is_err());
        assert!(container_body(&fixed, &spec, Role::Agent, Some("../x")).is_err());
        // Groups, scratch filesystems, kernel parameters, shared memory, limits, and the environment are exact too.
        assert!(mutated(Role::Display, &|b| h(b).group_add.as_mut().unwrap().push("0".into())).is_err());
        assert!(mutated(Role::Agent, &|b| h(b).group_add = None).is_err());
        assert!(
            mutated(Role::Browser, &|b| {
                h(b).tmpfs.as_mut().unwrap().insert("/tmp".into(), "rw,exec,size=256m".into());
            })
            .is_err()
        );
        assert!(
            mutated(Role::Agent, &|b| {
                h(b).tmpfs.as_mut().unwrap().insert("/run".into(), "rw,noexec,nosuid,nodev,size=1m".into());
            })
            .is_err()
        );
        assert!(
            mutated(Role::Display, &|b| h(b).sysctls =
                Some(HashMap::from([("net.ipv4.ip_forward".into(), "1".into())])))
            .is_err()
        );
        assert!(mutated(Role::Proxy, &|b| h(b).sysctls = None).is_err());
        assert!(mutated(Role::Browser, &|b| h(b).shm_size = Some(64 * 1024 * MIB)).is_err());
        assert!(mutated(Role::Display, &|b| h(b).shm_size = Some(MIB)).is_err());
        assert!(mutated(Role::Display, &|b| h(b).ulimits = Some(vec![])).is_err());
        assert!(mutated(Role::Display, &|b| h(b).ulimits.as_mut().unwrap()[1].hard = Some(1 << 20)).is_err());
        assert!(mutated(Role::Browser, &|b| h(b).ulimits = Some(ulimits(Role::Display))).is_err());
        for role in [Role::Display, Role::Browser, Role::Agent, Role::Proxy] {
            assert!(mutated(role, &|b| h(b).ulimits = None).is_err(), "{role:?} without its core limit");
            assert!(mutated(role, &|b| h(b).ulimits.as_mut().unwrap()[0].soft = Some(-1)).is_err(), "{role:?}");
        }
        assert!(mutated(Role::Display, &|b| h(b).oom_kill_disable = Some(true)).is_err());
        assert!(mutated(Role::Agent, &|b| b.env.as_mut().unwrap().push("LD_PRELOAD=/tmp/x.so".into())).is_err());
        assert!(mutated(Role::Browser, &|b| b.env.as_mut().unwrap().push("BROWSER_KIOSK=1".into())).is_err());
        assert!(mutated(Role::Proxy, &|b| b.env.as_mut().unwrap().pop().map(drop).unwrap()).is_err());
        assert!(mutated(Role::Display, &|b| b.env = None).is_err());
    }

    /// Every variant the controller builds: plain, DevTools, through an upstream, the bench fixture, the demo's walled
    /// garden (fixture only), kiosk, and a bench lane.
    fn variants() -> Vec<(&'static str, Fixed, Spec)> {
        let bench = |fixture_only, lanes| Bench {
            network: "easi-run-bench".into(),
            fixture: "fixture.bench=172.31.250.10".into(),
            spki_file: "/run/bench/spki".into(),
            lanes,
            fixture_only,
        };
        let spki = Some("47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU=".to_owned());
        let with = |bench: Option<Bench>| Fixed { bench, ..fixed() };
        let upstream = egress::Upstream::parse_url("http://alice:secret@proxy.example.net:8080").unwrap();
        vec![
            ("plain", fixed(), spec(false)),
            ("devtools", fixed(), spec(true)),
            ("upstream", fixed(), Spec { egress: Some(upstream), ..spec(false) }),
            ("bench fixture", with(Some(bench(false, vec![]))), Spec { bench_spki: spki.clone(), ..spec(false) }),
            (
                "walled garden",
                with(Some(bench(true, vec![]))),
                Spec { bench_spki: spki.clone(), walled: true, ..spec(false) },
            ),
            ("kiosk", with(Some(bench(false, vec![]))), Spec { kiosk: true, ..spec(false) }),
            (
                "bench lane",
                with(Some(bench(false, vec![Lane::parse("4,52/5,53/0").unwrap()]))),
                Spec { lane: Some(0), bench_spki: spki, ..spec(false) },
            ),
        ]
    }

    #[test]
    fn every_template_variant_passes_the_guard() {
        for (name, fixed, spec) in variants() {
            for role in Role::ORDER {
                let body = container_body(&fixed, &spec, role, Some(BROWSER_ID))
                    .unwrap_or_else(|why| panic!("{name} {role:?}: {why}"));
                assert_eq!(check_container(&fixed, &spec, role, Some(BROWSER_ID), &body), Ok(()), "{name} {role:?}");
            }
        }
    }

    #[test]
    fn the_guard_holds_mounts_limits_values_and_every_other_field_exactly() {
        fn h(b: &mut ContainerCreateBody) -> &mut HostConfig {
            b.host_config.as_mut().unwrap()
        }
        fn set(b: &mut ContainerCreateBody, var: &str) {
            let name = var.split_once('=').unwrap().0;
            let env = b.env.as_mut().unwrap();
            env.retain(|v| !v.starts_with(&format!("{name}=")));
            env.push(var.to_owned());
        }
        type Edit = Box<dyn Fn(&mut ContainerCreateBody)>;
        let edits: Vec<(Role, &str, Edit)> = vec![
            (
                Role::Browser,
                "a read-only mount made writable",
                Box::new(|b| h(b).mounts.as_mut().unwrap()[0].read_only = Some(false)),
            ),
            (
                Role::Agent,
                "a mount at another target",
                Box::new(|b| h(b).mounts.as_mut().unwrap()[0].target = Some("/run/display/agent".into())),
            ),
            (
                Role::Display,
                "a mount that copies the image's directory",
                Box::new(|b| h(b).mounts.as_mut().unwrap()[0].volume_options.as_mut().unwrap().no_copy = None),
            ),
            (Role::Agent, "a mount left out", Box::new(|b| drop(h(b).mounts.as_mut().unwrap().pop()))),
            (Role::Browser, "more processes", Box::new(|b| h(b).pids_limit = Some(4096))),
            (Role::Display, "no process limit", Box::new(|b| h(b).pids_limit = None)),
            (Role::Browser, "another start URL", Box::new(|b| set(b, "BROWSER_URL=https://other.example/"))),
            (Role::Proxy, "another client uid", Box::new(|b| set(b, "PROXY_CLIENT_UID=0"))),
            (Role::Display, "another output size", Box::new(|b| set(b, "DISPLAY_OUTPUT=1920x1080"))),
            (Role::Agent, "a repeated variable", Box::new(|b| b.env.as_mut().unwrap().push("HOME=/tmp".into()))),
            (Role::Display, "no log settings", Box::new(|b| h(b).log_config = None)),
            (
                Role::Display,
                "another log driver",
                Box::new(|b| h(b).log_config.as_mut().unwrap().typ = Some("json-file".into())),
            ),
            (
                Role::Browser,
                "annotations",
                Box::new(|b| h(b).annotations = Some(HashMap::from([("a".into(), "b".into())]))),
            ),
            (Role::Browser, "an OOM score", Box::new(|b| h(b).oom_score_adj = Some(-1000))),
            (Role::Proxy, "a restart policy", Box::new(|b| h(b).restart_policy = Some(Default::default()))),
            (Role::Proxy, "DNS servers", Box::new(|b| h(b).dns = Some(vec!["1.1.1.1".into()]))),
            (Role::Display, "an empty capability list", Box::new(|b| h(b).cap_add = Some(vec![]))),
            (
                Role::Display,
                "another label",
                Box::new(|b| drop(b.labels.as_mut().unwrap().insert("x".into(), "y".into()))),
            ),
            (
                Role::Proxy,
                "an endpoint alias",
                Box::new(|b| {
                    let endpoints = b.networking_config.as_mut().unwrap().endpoints_config.as_mut().unwrap();
                    endpoints.values_mut().next().unwrap().aliases = Some(vec!["proxy".into()]);
                }),
            ),
            (
                Role::Browser,
                "a network for the browser",
                Box::new(|b| b.networking_config = Some(NetworkingConfig::default())),
            ),
            (Role::Agent, "a working directory", Box::new(|b| b.working_dir = Some("/".into()))),
            (Role::Agent, "a healthcheck", Box::new(|b| b.healthcheck = Some(Default::default()))),
            (Role::Display, "attached output", Box::new(|b| b.attach_stdout = Some(true))),
        ];
        let (fixed, spec) = (fixed(), spec(false));
        for (role, what, edit) in edits {
            let mut body = container_body(&fixed, &spec, role, Some(BROWSER_ID)).unwrap();
            edit(&mut body);
            assert!(check_container(&fixed, &spec, role, Some(BROWSER_ID), &body).is_err(), "{role:?}: {what}");
        }
        // A kiosk browser outside the bench profile, and a body built for another session's request.
        let mut kiosk = container_body(&fixed, &spec, Role::Browser, None).unwrap();
        kiosk.env.as_mut().unwrap().push("BROWSER_KIOSK=1".into());
        assert!(check_container(&fixed, &Spec { kiosk: true, ..spec.clone() }, Role::Browser, None, &kiosk).is_err());
        let other = Spec { url: "https://other.example/".into(), ..spec.clone() };
        let body = container_body(&fixed, &other, Role::Browser, None).unwrap();
        assert!(check_container(&fixed, &spec, Role::Browser, None, &body).is_err());
    }

    #[test]
    fn an_upstream_reaches_only_the_proxys_standard_input() {
        let fixed = fixed();
        let mut spec = spec(false);
        spec.egress = Some(egress::Upstream::parse_url("http://alice:secret@proxy.example.net:8080").unwrap());
        for role in Role::ORDER {
            let body = container_body(&fixed, &spec, role, Some(BROWSER_ID)).unwrap();
            let shown = serde_json::to_string(&body).unwrap();
            assert!(
                !shown.contains("alice") && !shown.contains("secret") && !shown.contains("proxy.example"),
                "{role:?}"
            );
            let stdin = (body.open_stdin, body.stdin_once, body.attach_stdin);
            if role == Role::Proxy {
                assert_eq!(stdin, (Some(true), Some(true), Some(true)));
                assert_eq!(env_of(&body, "PROXY_UPSTREAM_STDIN").as_deref(), Some("1"));
            } else {
                assert_eq!(stdin, (Some(false), None, Some(false)), "{role:?}");
            }
            assert!(check_container(&fixed, &spec, role, Some(BROWSER_ID), &body).is_ok(), "{role:?}");
        }
        // Without an upstream, an open standard input is refused, for the proxy as for every other role.
        let direct = self::spec(false);
        for role in Role::ORDER {
            let mut body = container_body(&fixed, &direct, role, Some(BROWSER_ID)).unwrap();
            body.open_stdin = Some(true);
            assert!(check_container(&fixed, &direct, role, Some(BROWSER_ID), &body).is_err(), "{role:?}");
            let mut body = container_body(&fixed, &spec, role, Some(BROWSER_ID)).unwrap();
            body.tty = Some(true);
            assert!(check_container(&fixed, &spec, role, Some(BROWSER_ID), &body).is_err(), "{role:?}");
        }
    }
}
