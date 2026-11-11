//! The closed-set guard the controller applies right before every create: a container body outside the templates'
//! envelope is refused, so a template edit that widens anything fails at run time as well as in the tests. It states
//! that envelope independently of the template code, field by field: the image, user, labels, standard input,
//! environment (names and values), networks, mounts (source, target, read-only), groups, scratch filesystems, kernel
//! parameters, limits, placement, security options, namespaces, and log settings are exact, and any other field of
//! the body or its host config that is set at all is refused.

use std::collections::HashMap;

use bollard::exec::CreateExecOptions;
use bollard::models::{
    ContainerCreateBody, EndpointSettings, HostConfig, HostConfigCgroupnsModeEnum, HostConfigLogConfig, Mount,
    MountType, MountVolumeOptions, NetworkingConfig, ResourcesUlimits,
};

use crate::template::{
    AGENT_DIR, AGENT_EXEC, BROWSER_CDP_DIR, BROWSER_PROXY_DIR, BROWSER_WAYLAND_DIR, BUS_DIR, CDP_DIR, CDP_LISTEN,
    CDP_PORT, CPU_PERIOD_US, DISPLAY_AGENT_DIR, DISPLAY_BROWSER_DIR, DISPLAY_STREAM_DIR, Exec, Fixed,
    GID_AGENT_SOCKETS, GID_BUS, GID_CDP, GID_PROXY, GID_STREAMS, GID_WAYLAND, LABEL_INSTANCE, LABEL_ROLE,
    LABEL_SESSION, LAUNCHER, MIB, PROXY_DIR, RUNNER_ATTACH, RUNNER_CPUS_MILLI, RUNNER_HOME, RUNNER_MCP_DIR,
    RUNNER_PIDS, RUNNER_PROXY_DIR, Role, Spec, UID_ADMIN, UID_BROWSER, UID_CDP, UID_DISPLAY, UID_PROXY, UID_RUNNER,
    Vol, account_volume_name, agent_uid, agent_user, cdp_network_name, cdp_port_bindings, chat_directory, cpu_quota_us,
    network_name, placement, volume_name,
};

/// Every scratch filesystem's options start so: nothing written there runs, gains privilege, or is a device.
const SCRATCH: &str = "rw,noexec,nosuid,nodev";

/// `role`'s environment, every `NAME=value`, sorted. Values that come from the request (the URL, size, locale) or the
/// profile (the fixture, its SPKI) are what this session's spec and the controller's configuration say, exactly.
fn environment(fixed: &Fixed, spec: &Spec, role: Role) -> Vec<String> {
    let size = format!("{}x{}", spec.width, spec.height);
    let flag = |on: bool| if on { "1" } else { "0" }.to_owned();
    let mut vars: Vec<(&str, String)> = match role {
        Role::Display => vec![
            ("DISPLAY_OUTPUT", size),
            ("DISPLAY_BROWSER_SOCKET", format!("{DISPLAY_BROWSER_DIR}/wayland-0")),
            ("DISPLAY_BROWSER_UID", UID_BROWSER.to_string()),
            ("DISPLAY_AGENT_SOCKET", format!("{DISPLAY_AGENT_DIR}/wayland-0")),
            ("DISPLAY_INJECT_SOCKET", format!("{DISPLAY_AGENT_DIR}/cua-inject")),
            ("DISPLAY_AGENT_UID", agent_uid(spec.devtools).to_string()),
            ("DISPLAY_STREAM_SOCKET", format!("{DISPLAY_STREAM_DIR}/display.sock")),
            ("DISPLAY_ADMIN_UID", UID_ADMIN.to_string()),
            ("DISPLAY_AUDIO_FIFO", format!("{DISPLAY_BROWSER_DIR}/audio")),
        ]
        .into_iter()
        .chain(fixed.display_background.clone().map(|color| ("DISPLAY_BACKGROUND", color)))
        .chain(fixed.display_logo.clone().map(|logo| ("DISPLAY_LOGO", logo)))
        .collect(),
        Role::Proxy => vec![
            ("PROXY_LISTEN_UNIX", format!("{PROXY_DIR}/proxy.sock")),
            ("PROXY_CLIENT_UID", UID_BROWSER.to_string()),
        ],
        Role::Browser => vec![
            ("BROWSER_URL", spec.url.clone()),
            ("BROWSER_SIZE", size),
            ("BROWSER_TIMEZONE", spec.timezone.clone()),
            ("BROWSER_LANGUAGES", spec.languages.join(",")),
            ("BROWSER_DEVTOOLS", flag(spec.devtools)),
            ("BROWSER_PROXY_SOCKET", format!("{BROWSER_PROXY_DIR}/proxy.sock")),
            ("BROWSER_MAX_TABS", fixed.browser_max_tabs.to_string()),
        ],
        Role::Agent => vec![
            ("HOME", "/tmp".to_owned()),
            ("WAYLAND_DISPLAY", format!("{AGENT_DIR}/wayland-0")),
            ("CUA_INJECT_SOCKET", format!("{AGENT_DIR}/cua-inject")),
            ("CUA_DRIVER_RS_ENABLE_WAYLAND", "1".to_owned()),
            ("DBUS_SESSION_BUS_ADDRESS", format!("unix:path={BUS_DIR}/session")),
            ("AGENT_DEVTOOLS", flag(spec.devtools)),
        ],
        // The upstream arrives on standard input, never here; nothing else configures a check.
        Role::Check => vec![("PROXY_CHECK", "1".to_owned())],
        Role::RunnerProxy => {
            vec![("PROXY_LISTEN_UNIX", format!("{PROXY_DIR}/proxy.sock")), ("PROXY_CLIENT_UID", UID_RUNNER.to_string())]
        }
        Role::Cdp => {
            vec![("PROXY_RELAY_LISTEN", CDP_LISTEN.to_owned()), ("PROXY_RELAY_SOCKET", format!("{CDP_DIR}/cdp.sock"))]
        }
        Role::Runner => vec![
            ("HOME", RUNNER_HOME.to_owned()),
            ("PATH", format!("{RUNNER_HOME}/.local/bin:/usr/local/bin:/usr/bin:/bin")),
            ("TERM", "xterm-256color".to_owned()),
            ("LANG", "C.UTF-8".to_owned()),
            ("HTTPS_PROXY", "http://127.0.0.1:3128".to_owned()),
            ("HTTP_PROXY", "http://127.0.0.1:3128".to_owned()),
            ("NO_PROXY", "localhost,127.0.0.1,::1".to_owned()),
            ("RUNNER_PRODUCT", spec.chat.as_ref().map_or("", |chat| chat.product.name()).to_owned()),
            ("RUNNER_PLACE", spec.chat.as_ref().map_or("", |chat| chat.place.as_str()).to_owned()),
        ],
    };
    match role {
        Role::Proxy => {
            // The upstream itself arrives on standard input, never here.
            if spec.egress.is_some() {
                vars.push(("PROXY_UPSTREAM_STDIN", "1".to_owned()));
            }
            if let Some(bench) = &fixed.bench {
                vars.push(("PROXY_BENCH_FIXTURE", bench.fixture.clone()));
                if bench.fixture_only {
                    vars.push(("PROXY_FIXTURE_ONLY", "1".to_owned()));
                }
            }
        }
        Role::Browser => {
            if let Some(spki) = &spec.bench_spki {
                vars.push(("BROWSER_BENCH_SPKI", spki.clone()));
            }
            // Kiosk mode is the bench profile's alone.
            if spec.kiosk && fixed.bench.is_some() {
                vars.push(("BROWSER_KIOSK", "1".to_owned()));
            }
            if spec.profile {
                vars.push(("BROWSER_PROFILE", "1".to_owned()));
            }
            if spec.cdp {
                vars.push(("BROWSER_CDP_SOCKET", format!("{BROWSER_CDP_DIR}/cdp.sock")));
            }
        }
        Role::Display | Role::Agent | Role::Check | Role::Runner | Role::RunnerProxy | Role::Cdp => {}
    }
    let mut vars: Vec<String> = vars.into_iter().map(|(name, value)| format!("{name}={value}")).collect();
    vars.sort_unstable();
    vars
}

/// `role`'s mounts, in order: named volumes only, each at its one target, read-only wherever the role only reads.
fn mounts(fixed: &Fixed, spec: &Spec, role: Role) -> Vec<Mount> {
    let own = |vol, target: &'static str, read_only: bool| {
        (volume_name(&fixed.instance, &spec.session, vol), target, read_only, None)
    };
    let wanted = match role {
        Role::Display => vec![
            own(Vol::Wayland, DISPLAY_BROWSER_DIR, false),
            own(Vol::Agent, DISPLAY_AGENT_DIR, false),
            // The long-lived streams volume, narrowed to this session's subpath.
            (fixed.streams_volume.clone(), DISPLAY_STREAM_DIR, false, Some(spec.session.to_string())),
        ],
        Role::Proxy => vec![own(Vol::Proxy, PROXY_DIR, false)],
        Role::Browser if spec.cdp => vec![
            own(Vol::Wayland, BROWSER_WAYLAND_DIR, true),
            own(Vol::Bus, BUS_DIR, false),
            own(Vol::Proxy, BROWSER_PROXY_DIR, true),
            own(Vol::Cdp, BROWSER_CDP_DIR, false),
        ],
        Role::Browser => vec![
            own(Vol::Wayland, BROWSER_WAYLAND_DIR, true),
            own(Vol::Bus, BUS_DIR, false),
            own(Vol::Proxy, BROWSER_PROXY_DIR, true),
        ],
        Role::Cdp => vec![own(Vol::Cdp, CDP_DIR, true)],
        Role::Agent => vec![own(Vol::Agent, AGENT_DIR, true), own(Vol::Bus, BUS_DIR, true)],
        Role::Check => vec![],
        Role::RunnerProxy => vec![own(Vol::RunnerProxy, PROXY_DIR, false)],
        Role::Runner => vec![
            (
                account_volume_name(&fixed.instance, spec.chat.as_ref().map_or("", |chat| &chat.account)),
                RUNNER_HOME,
                false,
                None,
            ),
            own(Vol::RunnerProxy, RUNNER_PROXY_DIR, true),
            (fixed.streams_volume.clone(), RUNNER_MCP_DIR, true, Some(chat_directory(&spec.session))),
        ],
    };
    wanted
        .into_iter()
        .map(|(source, target, read_only, subpath)| Mount {
            typ: Some(MountType::VOLUME),
            source: Some(source),
            target: Some(target.to_owned()),
            read_only: Some(read_only),
            // Only an account's home takes the image's directory, for its ownership, when it is new.
            volume_options: Some(MountVolumeOptions {
                no_copy: Some(target != RUNNER_HOME),
                subpath,
                ..Default::default()
            }),
            ..Default::default()
        })
        .collect()
}

/// `role`'s networks: the session's egress network for its proxy and a chat's proxy (and the fixture's, in the bench
/// profile), a check's own network made the same way (and the fixture's), the CDP relay's own network, and nothing for
/// the others.
fn networks(fixed: &Fixed, spec: &Spec, role: Role) -> Option<NetworkingConfig> {
    if role == Role::Cdp {
        let endpoints =
            HashMap::from([(cdp_network_name(&fixed.instance, &spec.session), EndpointSettings::default())]);
        return Some(NetworkingConfig { endpoints_config: Some(endpoints) });
    }
    if !matches!(role, Role::Proxy | Role::Check | Role::RunnerProxy) {
        return None;
    }
    let mut endpoints = HashMap::from([(network_name(&fixed.instance, &spec.session), EndpointSettings::default())]);
    if let Some(bench) = &fixed.bench {
        endpoints.insert(bench.network.clone(), EndpointSettings::default());
    }
    Some(NetworkingConfig { endpoints_config: Some(endpoints) })
}

/// Whether `host` holds exactly `role`'s supplementary groups, scratch filesystems, kernel parameters, shared memory,
/// and descriptor limits.
fn exact_extras(spec: &Spec, role: Role, host: &HostConfig) -> bool {
    let groups: &[u32] = match role {
        Role::Browser if spec.cdp => &[GID_WAYLAND, GID_BUS, GID_PROXY, GID_CDP],
        Role::Cdp => &[GID_CDP],
        Role::Display => &[GID_WAYLAND, GID_AGENT_SOCKETS, GID_STREAMS],
        Role::Proxy | Role::RunnerProxy => &[GID_PROXY],
        Role::Runner => &[GID_STREAMS, GID_PROXY],
        Role::Browser => &[GID_WAYLAND, GID_BUS, GID_PROXY],
        Role::Agent => &[GID_BUS, GID_AGENT_SOCKETS],
        Role::Check => &[],
    };
    let owned = |size: &str| format!("size={size},uid={UID_BROWSER},gid={UID_BROWSER},mode=0700");
    let scratch = match role {
        Role::Display | Role::Agent => vec![("/tmp", "size=64m".to_owned())],
        Role::Proxy | Role::RunnerProxy => vec![("/tmp", "size=8m".to_owned())],
        Role::Runner => vec![("/tmp", "size=256m".to_owned())],
        Role::Check | Role::Cdp => vec![],
        Role::Browser => vec![
            ("/tmp", "size=256m".to_owned()),
            ("/home/browser", owned("16g")),
            (display_protocol::browser::DOWNLOAD_DIR, owned("256m")),
            (display_protocol::browser::UPLOAD_DIR, owned("1g")),
        ],
    };
    let groups: Vec<String> = groups.iter().map(u32::to_string).collect();
    let scratch: HashMap<String, String> =
        scratch.into_iter().map(|(path, options)| (path.to_owned(), format!("{SCRATCH},{options}"))).collect();
    let sysctls = matches!(role, Role::Proxy | Role::Check | Role::RunnerProxy | Role::Cdp)
        .then(|| HashMap::from([("net.ipv4.ip_forward".to_owned(), "0".to_owned())]));
    let shm = (role == Role::Browser).then_some(512 * MIB);
    // No core file anywhere, soft and hard; the display also holds its descriptors (display/src/limits.rs).
    let ulimits: &[(&str, i64)] = match role {
        Role::Display => &[("core", 1), ("nofile", 32768)],
        Role::Proxy | Role::Browser | Role::Agent | Role::Check | Role::Runner | Role::RunnerProxy | Role::Cdp => {
            &[("core", 1)]
        }
    };
    let ulimits: Vec<ResourcesUlimits> = ulimits
        .iter()
        .map(|&(name, value)| ResourcesUlimits { name: Some(name.to_owned()), soft: Some(value), hard: Some(value) })
        .collect();
    host.group_add.as_ref() == Some(&groups)
        && host.tmpfs.as_ref() == Some(&scratch)
        && host.sysctls == sysctls
        && host.shm_size == shm
        && host.ulimits.as_ref() == Some(&ulimits)
}

/// Every container's logs: the local driver, rotated, so a noisy page cannot fill the host's disk.
fn log_config() -> HostConfigLogConfig {
    HostConfigLogConfig {
        typ: Some("local".to_owned()),
        config: Some(HashMap::from([
            ("max-size".to_owned(), "10m".to_owned()),
            ("max-file".to_owned(), "3".to_owned()),
        ])),
    }
}

/// The closed-set guard. It refuses any body outside the templates' envelope.
pub fn check_container(
    fixed: &Fixed,
    spec: &Spec,
    role: Role,
    browser_id: Option<&str>,
    body: &ContainerCreateBody,
) -> Result<(), String> {
    let refuse = |what: &str| Err(format!("{} template refused: {what}", role.name()));
    let image = match role {
        Role::Display => &fixed.images.display,
        Role::Proxy | Role::Check | Role::RunnerProxy | Role::Cdp => &fixed.images.proxy,
        Role::Runner => &fixed.images.runner,
        Role::Browser => &fixed.images.browser,
        Role::Agent => &fixed.images.agent,
    };
    if body.image.as_ref() != Some(image) {
        return refuse("an image other than the pinned one");
    }
    let user = match role {
        Role::Display => format!("{UID_DISPLAY}:{UID_DISPLAY}"),
        Role::Proxy | Role::Check | Role::RunnerProxy => format!("{UID_PROXY}:{UID_PROXY}"),
        Role::Runner => format!("{UID_RUNNER}:{UID_RUNNER}"),
        Role::Browser => format!("{UID_BROWSER}:{UID_BROWSER}"),
        Role::Agent => agent_user(spec.devtools),
        Role::Cdp => format!("{UID_CDP}:{UID_CDP}"),
    };
    let walled = spec.walled || fixed.bench.as_ref().is_some_and(|b| b.fixture_only);
    if (role == Role::Cdp || spec.cdp) && (!spec.cdp || !spec.devtools || walled) {
        return refuse("CDP outside a session in DevTools mode that asked for it, or in the walled garden");
    }
    if body.user.as_ref() != Some(&user) {
        return refuse("another user");
    }
    let labels = HashMap::from([
        (LABEL_INSTANCE.to_owned(), fixed.instance.clone()),
        (LABEL_SESSION.to_owned(), spec.session.to_string()),
        (LABEL_ROLE.to_owned(), role.name().to_owned()),
    ]);
    if body.labels.as_ref() != Some(&labels) {
        return refuse("labels other than the instance, session, and role");
    }
    // Standard input stays closed but for a proxy given an upstream, or a check, to which it arrives there once; no
    // output is attached.
    let stdin = (body.open_stdin, body.stdin_once, body.attach_stdin);
    let expected_stdin = if (role == Role::Proxy && spec.egress.is_some()) || role == Role::Check {
        (Some(true), Some(true), Some(true))
    } else {
        (Some(false), None, Some(false))
    };
    if stdin != expected_stdin
        || (body.attach_stdout, body.attach_stderr, body.tty) != (Some(false), Some(false), Some(false))
    {
        return refuse("standard input, output, or a terminal outside the template");
    }
    let mut env = body.env.clone().unwrap_or_default();
    env.sort_unstable();
    if env != environment(fixed, spec, role) {
        return refuse("an environment outside the template");
    }
    if body.networking_config != networks(fixed, spec, role) {
        return refuse("a network the template does not join");
    }
    // The CDP relay's one port, and nothing else's.
    if body.exposed_ports != (role == Role::Cdp).then(|| vec![CDP_PORT.to_owned()]) {
        return refuse("an exposed port outside the template");
    }
    // Whatever the checks above did not name (a command, working directory, healthcheck, stop signal, anonymous volume,
    // ...) is refused by being set at all; exposed ports are the CDP relay's one, checked above.
    let rest = ContainerCreateBody {
        image: None,
        user: None,
        labels: None,
        open_stdin: None,
        stdin_once: None,
        attach_stdin: None,
        attach_stdout: None,
        attach_stderr: None,
        tty: None,
        env: None,
        networking_config: None,
        exposed_ports: None,
        host_config: None,
        ..body.clone()
    };
    if rest != ContainerCreateBody::default() {
        return refuse("a container setting outside the template");
    }
    let Some(host) = body.host_config.as_ref() else { return refuse("no host config") };
    check_host(fixed, spec, role, browser_id, host).or_else(|what| refuse(&what))
}

/// The host config's part of the guard; an error says what is outside the template.
fn check_host(
    fixed: &Fixed,
    spec: &Spec,
    role: Role,
    browser_id: Option<&str>,
    host: &HostConfig,
) -> Result<(), String> {
    let refuse = |what: &str| Err(what.to_owned());
    if host.privileged != Some(false) {
        return refuse("privileged");
    }
    if host.cap_drop.as_deref() != Some(&["ALL".to_owned()][..]) {
        return refuse("capabilities");
    }
    // The agent, in the browser's PID namespace, has no init of its own.
    if host.readonly_rootfs != Some(true) || host.init != (role != Role::Agent).then_some(true) {
        return refuse("a writable root or an init outside the template");
    }
    if !exact_extras(spec, role, host) {
        return refuse("groups, scratch filesystems, kernel parameters, shared memory, or descriptor limits");
    }
    let mut opts = vec!["no-new-privileges:true".to_owned()];
    if role == Role::Browser {
        opts.push(format!("seccomp={}", fixed.browser_seccomp));
    }
    if host.security_opt.as_ref() != Some(&opts) {
        return refuse("security options outside the template");
    }
    if host.mounts.as_ref() != Some(&mounts(fixed, spec, role)) {
        return refuse("mounts other than the template's volumes, targets, and read-only flags");
    }
    let joined = browser_id.map(|id| format!("container:{id}"));
    let expected_network = match role {
        Role::Display | Role::Browser | Role::Runner => Some("none".to_owned()),
        Role::Proxy | Role::Check | Role::RunnerProxy => Some(network_name(&fixed.instance, &spec.session)),
        Role::Cdp => Some(cdp_network_name(&fixed.instance, &spec.session)),
        Role::Agent if spec.devtools => joined.clone(),
        Role::Agent => Some("none".to_owned()),
    };
    if host.network_mode != expected_network {
        return refuse("another network mode");
    }
    let expected_pid = if role == Role::Agent { joined } else { None };
    if host.pid_mode != expected_pid {
        return refuse("another pid namespace");
    }
    if host.ipc_mode.as_deref() != Some("private") || host.cgroupns_mode != Some(HostConfigCgroupnsModeEnum::PRIVATE) {
        return refuse("a shared IPC or cgroup namespace");
    }
    if host.publish_all_ports != Some(false) {
        return refuse("published ports");
    }
    // The CDP relay's one port, on the host's loopback alone; no other role publishes any.
    if host.port_bindings != (role == Role::Cdp).then(cdp_port_bindings) {
        return refuse("a published port outside the template");
    }
    let (cpus_milli, memory_mib, pids) = match role {
        Role::Display => (crate::template::display_cpus_milli(spec), 512, 256),
        Role::Proxy | Role::RunnerProxy => (500, 128, 256),
        Role::Runner => (RUNNER_CPUS_MILLI, fixed.chat_memory_mib, RUNNER_PIDS),
        Role::Browser => (fixed.browser.cpus_milli, fixed.browser.memory_mib, 2048),
        Role::Agent => (500, 512, 512),
        Role::Check | Role::Cdp => (250, 64, 64),
    };
    if host.cpu_period != Some(CPU_PERIOD_US)
        || host.cpu_quota != Some(cpu_quota_us(cpus_milli))
        || host.memory != Some(memory_mib * MIB)
        || host.memory_swap != host.memory
        || host.pids_limit != Some(pids)
    {
        return refuse("resource limits");
    }
    if (host.cpuset_cpus.clone(), host.cpuset_mems.clone()) != placement(fixed, spec, role)?.unzip() {
        return refuse("a CPU or memory placement outside the bench lane");
    }
    if host.log_config != Some(log_config()) {
        return refuse("log settings outside the template");
    }
    // Every other host setting (binds, devices, host namespaces, a runtime, restart policy, OOM score, annotations,
    // DNS, ...) is refused by being set at all; published ports are the CDP relay's one, checked above.
    let rest = HostConfig {
        privileged: None,
        cap_drop: None,
        readonly_rootfs: None,
        init: None,
        group_add: None,
        tmpfs: None,
        sysctls: None,
        shm_size: None,
        ulimits: None,
        security_opt: None,
        mounts: None,
        network_mode: None,
        pid_mode: None,
        ipc_mode: None,
        cgroupns_mode: None,
        publish_all_ports: None,
        port_bindings: None,
        cpu_period: None,
        cpu_quota: None,
        memory: None,
        memory_swap: None,
        pids_limit: None,
        cpuset_cpus: None,
        cpuset_mems: None,
        log_config: None,
        ..host.clone()
    };
    if rest != HostConfig::default() {
        return refuse("a host setting outside the template");
    }
    Ok(())
}

/// The closed-set guard of an exec, right before it is created: exactly one of the [`Exec`] set, its command and user
/// stated here independently of the template, standard streams without a terminal or privileges, and nothing else set
/// (no environment, working directory or detach keys). A profile exec runs only in a session started from a profile, and
/// a chat's only while it has one, the only one with a terminal.
pub fn check_exec(spec: &Spec, exec: Exec, options: &CreateExecOptions<String>) -> Result<(), String> {
    let refuse = |what: &str| Err(format!("exec refused: {what}"));
    let (command, user, reads): (Vec<&str>, String, bool) = match exec {
        Exec::Agent => (vec![AGENT_EXEC], agent_user(spec.devtools), true),
        Exec::ProfileRestore => (vec![LAUNCHER, "profile-restore"], format!("{UID_BROWSER}:{UID_BROWSER}"), true),
        Exec::ProfileSave => (vec![LAUNCHER, "profile-save"], format!("{UID_BROWSER}:{UID_BROWSER}"), false),
        Exec::Chat => (vec![RUNNER_ATTACH], format!("{UID_RUNNER}:{UID_RUNNER}"), true),
    };
    if matches!(exec, Exec::ProfileRestore | Exec::ProfileSave) && !spec.profile {
        return refuse("a profile exec in a session without a profile");
    }
    if exec == Exec::Chat && spec.chat.is_none() {
        return refuse("a chat exec in a session without a chat");
    }
    let wanted = CreateExecOptions::<String> {
        attach_stdin: Some(reads),
        attach_stdout: Some(true),
        attach_stderr: Some(true),
        tty: Some(exec == Exec::Chat),
        privileged: Some(false),
        user: Some(user),
        cmd: Some(command.into_iter().map(str::to_owned).collect()),
        ..Default::default()
    };
    if *options != wanted {
        return refuse("a command, user, stream, or setting outside the closed set");
    }
    Ok(())
}
