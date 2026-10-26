//! The security requirements of docs/design.md §4, proven against the running stack. It inspects and probes containers
//! through the Docker CLI, and runs its probes (`net-probe`, `wl-globals`, built from this crate) in throwaway
//! containers that join a session container's namespaces or mount its volumes.

mod common;

use std::collections::BTreeSet;
use std::time::Duration;

use common::{ADMIN_CONTAINER, Admin, CONTROLLER_CONTAINER, INSTANCE, docker};
use serde_json::{Value, json};

const PROBE_IMAGE: &str =
    "docker.io/library/debian:trixie-slim@sha256:a29215f6a35e51e22adffa17f89e9d2ef06214e64a2bad10d765c46aea49f11f";

fn inspect(name: &str) -> Value {
    let (ok, text) = docker(&["inspect", name]);
    assert!(ok, "inspect {name}: {text}");
    serde_json::from_str::<Value>(&text).expect("inspect output")[0].clone()
}

fn binary(name: &str) -> String {
    // Where cargo puts it: the target directory it was given, else the workspace's.
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../target"));
    // Built every time: cargo returns at once when the probe is fresh, and never hands over one older than its source.
    let status = std::process::Command::new(env!("CARGO"))
        .args(["build", "--release", "-p", "display-tests", "--bin", name])
        .status()
        .expect("cargo builds the probe");
    assert!(status.success(), "cargo build --bin {name}");
    target.join("release").join(name).canonicalize().unwrap().display().to_string()
}

/// The test upstream the egress tests add (compose.bench.yaml), and its address on the fixture network.
const UPSTREAM: &str = "easi-run-bench-upstream-1";
const UPSTREAM_URL: &str = "http://10.250.250.20:3128";

/// Now, in milliseconds since the Unix epoch.
fn unix_ms_now() -> i64 {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).expect("a clock after 1970");
    t.as_millis() as i64
}

/// A proxy log line's `ts` (`2026-10-25T21:06:42.865Z`), in milliseconds since the Unix epoch.
fn rfc3339_ms(ts: &str) -> Option<i64> {
    let number = |range: std::ops::Range<usize>| ts.get(range)?.parse::<i64>().ok();
    let (y, m, d) = (number(0..4)?, number(5..7)?, number(8..10)?);
    let (hh, mm, ss, ms) = (number(11..13)?, number(14..16)?, number(17..19)?, number(20..23)?);
    // Days from the civil date (Howard Hinnant's algorithm).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let days = era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468;
    Some(((days * 24 + hh) * 60 + mm) * 60_000 + ss * 1000 + ms)
}

/// Now, as a Unix timestamp `docker logs --since` takes.
fn now() -> String {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).expect("a clock after 1970");
    format!("{}.{:09}", t.as_secs(), t.subsec_nanos())
}

/// What `container` logged since `since` ([`now`], or `0` for everything), one line each: polled for up to 30 s until
/// `done` holds of the lines.
async fn logged_since(container: &str, since: &str, done: impl Fn(&[String]) -> bool) -> Vec<String> {
    let mut lines = Vec::new();
    common::until!(60, Duration::from_millis(500), {
        let (ok, text) = docker(&["logs", "--since", since, container]);
        assert!(ok, "docker logs {container}: {text}");
        lines = text.lines().map(str::to_owned).collect();
        done(&lines).then_some(())
    });
    lines
}

/// Every process of `name`, including init, runs as a non-root uid.
fn assert_no_root_process(name: &str) {
    let (ok, top) = docker(&["top", name, "-o", "pid,uid,comm"]);
    assert!(ok, "{top}");
    let root = |l: &&str| matches!(l.split_whitespace().nth(1), Some("0" | "root"));
    assert!(!top.lines().skip(1).any(|l| root(&l)), "{name} has a root process: {top}");
}

fn roles() -> [&'static str; 4] {
    ["display", "proxy", "browser", "agent"]
}

async fn session(admin: &Admin, devtools: bool) -> String {
    let id = admin.create_with(json!({"url": "https://fixture.bench/article.html", "devtools": devtools})).await;
    // The browser maps its window and the agent can see it.
    admin.wait_windows(&id, &format!("session {id}'s window"), |windows| (!windows.is_empty()).then_some(())).await;
    id
}

/// The container's main process makes no core dump: its core limit is 1, soft and hard, as the kernel holds it, which
/// the kernel takes as no core file and nothing piped to a collector the host's core pattern names.
fn assert_no_core_files(container: &Value, name: &str) {
    let pid = container["State"]["Pid"].as_u64().expect("a running container's pid");
    let limits = std::fs::read_to_string(format!("/proc/{pid}/limits")).expect("its limits");
    let core = limits.lines().find(|l| l.starts_with("Max core file size")).expect("a core limit");
    let fields: Vec<&str> = core.split_whitespace().collect();
    assert_eq!(fields[4..6], ["1", "1"], "{name} may dump core: {core}");
}

/// Items 1 and 2: no root, no core file, distinct identities, and only the Wayland directories shared between browser
/// and display, read-only on the browser's side (controller/tests/docker.rs asserts the session containers' capabilities,
/// devices and read-only roots).
#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn containers_run_unprivileged_with_distinct_identities() {
    let admin = Admin::login().await;
    for devtools in [false, true] {
        let id = session(&admin, devtools).await;
        let mut users = Vec::new();
        for role in roles() {
            let name = format!("{INSTANCE}-{id}-{role}");
            let c = inspect(&name);
            assert_no_core_files(&c, &name);
            let user = c["Config"]["User"].as_str().unwrap().to_owned();
            assert!(!user.starts_with("0:") && !user.is_empty(), "{name} runs as {user}");
            assert_no_root_process(&name);
            users.push((role, user));
        }
        let expect = [
            ("display", "10002:10002"),
            ("proxy", "10003:10003"),
            ("browser", "10001:10001"),
            ("agent", if devtools { "10001:10001" } else { "10005:10005" }),
        ];
        for (role, user) in expect {
            assert_eq!(users.iter().find(|(r, _)| *r == role).unwrap().1, user, "{role}");
        }
        let mounts = |role: &str| -> BTreeSet<(String, bool)> {
            inspect(&format!("{INSTANCE}-{id}-{role}"))["Mounts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|m| (m["Name"].as_str().unwrap_or_default().to_owned(), m["RW"].as_bool().unwrap()))
                .collect()
        };
        let display: BTreeSet<String> = mounts("display").into_iter().map(|(n, _)| n).collect();
        let browser = mounts("browser");
        let shared: Vec<&(String, bool)> = browser.iter().filter(|(n, _)| display.contains(n)).collect();
        assert_eq!(
            shared,
            [&(format!("{INSTANCE}-{id}-wayland"), false)],
            "browser and display share only the Wayland directory"
        );
        admin.stop(&id).await;
    }
    for service in [ADMIN_CONTAINER, CONTROLLER_CONTAINER] {
        let c = inspect(service);
        assert_eq!(c["HostConfig"]["CapDrop"], json!(["ALL"]), "{service}");
        assert_eq!(c["HostConfig"]["ReadonlyRootfs"], true, "{service}");
        // They write no core file either (compose.yaml).
        assert_no_core_files(&c, service);
        assert_no_root_process(service);
    }
    // Only the controller holds the Docker socket.
    let admin_mounts = inspect(ADMIN_CONTAINER)["Mounts"].to_string();
    assert!(!admin_mounts.contains("docker.sock"), "the admin has Docker access");
    assert!(inspect(CONTROLLER_CONTAINER)["Mounts"].to_string().contains("docker.sock"));
}

/// Runs `command` in a test-only container of the probe image, started with Docker's `options`.
fn in_probe(options: &[&str], command: &[&str]) -> (bool, String) {
    docker(&[&["run", "--rm"], options, &[PROBE_IMAGE], command].concat())
}

/// Runs `command` as the browser's uid in the browser container's network namespace or, with `chrome`, in Chrome's own
/// (the one its launcher made, browser/launch netns.rs), entered through the container's shared process namespace
/// from this test-only container.
fn in_browser(id: &str, chrome: bool, mounts: &[&str], command: &[&str]) -> (bool, String) {
    let netns = format!("container:{INSTANCE}-{id}-browser");
    let mut args = vec!["run", "--rm", "--network", netns.as_str()];
    for mount in mounts {
        args.extend(["-v", mount]);
    }
    let enter = "pid=$(for p in /proc/[0-9]*; do [ \"$(cat $p/comm 2>/dev/null)\" = browser-launch ] && echo ${p#/proc/}; done); \
                 exec nsenter -t $pid -n -S 10001 -G 10001 \"$@\"";
    if chrome {
        args.extend(["--pid", netns.as_str(), "--cap-add", "SYS_ADMIN", "--cap-add", "SYS_PTRACE"]);
        args.extend(["--security-opt", "seccomp=unconfined", PROBE_IMAGE, "sh", "-c", enter, "sh"]);
    } else {
        args.extend(["--user", "10001:10001", PROBE_IMAGE]);
    }
    args.extend(command);
    docker(&args)
}

/// Item 3: socket inventories of every session container's network namespace.
#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn socket_inventories_show_only_the_intended_endpoints() {
    let admin = Admin::login().await;
    let id = session(&admin, false).await;
    let inventory = |role: &str| -> (String, String, String) {
        let script = "for f in tcp tcp6; do tail -n +2 /proc/net/$f | awk '$4==\"0A\"{print $2}'; done; echo ---; \
                      for f in udp udp6; do tail -n +2 /proc/net/$f; done; echo ---; \
                      tail -n +2 /proc/net/unix | awk '$6==\"01\" && $8!=\"\"{print $8}' | sort -u";
        let (ok, text) = in_probe(&["--network", &format!("container:{INSTANCE}-{id}-{role}")], &["sh", "-c", script]);
        assert!(ok, "{text}");
        let mut parts = text.split("---\n");
        (
            parts.next().unwrap().trim().to_owned(),
            parts.next().unwrap().trim().to_owned(),
            parts.next().unwrap_or("").trim().to_owned(),
        )
    };
    // The display has no network: no TCP or UDP at all, and only its four sockets.
    let (tcp, udp, unix) = inventory("display");
    assert!(tcp.is_empty() && udp.is_empty(), "display: tcp {tcp:?} udp {udp:?}");
    let unix: BTreeSet<&str> = unix.lines().collect();
    assert_eq!(
        unix,
        BTreeSet::from([
            "/run/display/agent/cua-inject",
            "/run/display/agent/wayland-0",
            "/run/display/browser/wayland-0",
            "/run/display/stream/display.sock"
        ])
    );
    // The admin, with the direct path on (as in the bench stack): one UDP socket, its port on every address
    // (0.0.0.0:9443, as /proc/net/udp writes it), and no other but Docker's embedded resolver on 127.0.0.11.
    let (ok, text) = in_probe(
        &["--network", "container:easi-run-bench-admin-1"],
        &["sh", "-c", "tail -n +2 /proc/net/udp /proc/net/udp6 2>/dev/null | awk 'NF > 5 {print $2}'"],
    );
    assert!(ok, "{text}");
    let sockets: Vec<&str> = text.split_whitespace().filter(|s| !s.starts_with("0B00007F:")).collect();
    assert_eq!(sockets, ["00000000:24E3"], "the admin's UDP sockets: {text}");
    // The browser container's namespace holds nothing at all; Chrome's own has only the proxy forwarder on loopback, no
    // UDP socket, and as links loopback and the veth pair, whose IPv4 address is the only one beside loopback's. A host
    // with tunnel modules loaded puts fallback devices (`tunl0`, `sit0`, ...) in every new namespace: other links may
    // exist, but none has an address or a route, so none carries anything.
    let script = "for f in tcp tcp6; do tail -n +2 /proc/net/$f | awk '$4==\"0A\"{print $2}'; done; echo ---; \
                  for f in udp udp6; do tail -n +2 /proc/net/$f; done; echo ---; \
                  awk 'NR>2{print $1}' /proc/net/dev | sort; echo ---; cat /proc/net/if_inet6; echo ---; \
                  tail -n +2 /proc/net/route | awk '{print $1, $2}'; echo ---; \
                  awk '/\\|--/{a=$2} /\\/32 host LOCAL/ && a !~ /^127\\./{print a}' /proc/net/fib_trie | sort -u";
    let namespace = |chrome: bool| -> Vec<String> {
        let (ok, text) = in_browser(&id, chrome, &[], &["sh", "-c", script]);
        assert!(ok, "{text}");
        text.split("---\n").map(|p| p.trim().to_owned()).collect()
    };
    let links = |parts: &[String]| -> BTreeSet<String> { parts[2].lines().map(str::to_owned).collect() };
    let outer = namespace(false);
    assert!(outer[0].is_empty() && outer[1].is_empty(), "the browser container's sockets: {outer:?}");
    assert!(links(&outer).contains("lo:"), "the browser container's links: {}", outer[2]);
    assert!(!outer[2].contains("eth"), "the browser container's namespace has a link: {}", outer[2]);
    assert!(outer[3].lines().all(|l| l.ends_with(" lo")), "an IPv6 address beyond loopback: {}", outer[3]);
    assert!(outer[4].is_empty() && outer[5].is_empty(), "the browser container's routes and addresses: {outer:?}");
    let parts = namespace(true);
    assert_eq!(parts[0], "0100007F:0C38", "Chrome's listeners");
    assert!(parts[1].is_empty(), "Chrome's namespace has a UDP socket: {}", parts[1]);
    let chrome_links = links(&parts);
    assert!(["lo:", "eth0:", "eth1:"].iter().all(|l| chrome_links.contains(*l)), "Chrome's links: {}", parts[2]);
    assert!(parts[3].lines().all(|l| l.ends_with(" lo")), "an IPv6 address beyond loopback: {}", parts[3]);
    assert_eq!(parts[4], "eth0 0001A8C0", "Chrome's routes: only the link's own subnet");
    assert_eq!(parts[5], "192.168.1.100", "Chrome's IPv4 addresses beyond loopback");
    // The proxy listens on nothing but its socket (Docker's resolver in its egress network aside).
    let (tcp, _, _) = inventory("proxy");
    assert!(tcp.lines().all(|l| l.starts_with("0B00007F:")), "proxy listeners beyond Docker's resolver: {tcp}");
    admin.stop(&id).await;
}

/// Item 4: from the browser's namespace nothing is reachable directly; through the proxy, private and loopback
/// destinations (also by a name that resolves to one) are refused, and a public one is admitted.
#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn the_browser_reaches_nothing_but_the_proxy() {
    let admin = Admin::login().await;
    let id = session(&admin, false).await;
    let probe = binary("net-probe");
    let gateway = docker(&["network", "inspect", "-f", "{{range .IPAM.Config}}{{.Gateway}}{{end}}", "bridge"]).1;
    let gateway = format!("{}:80", gateway.trim());
    let direct = [
        "tcp",
        "1.1.1.1:443",
        "tcp",
        "93.184.215.14:80",
        "tcp",
        &gateway,
        "tcp",
        "10.250.250.10:443",
        "tcp",
        "172.17.0.1:22",
        "tcp",
        "169.254.169.254:80",
        "udp",
        "1.1.1.1:53",
        "udp",
        "8.8.8.8:53",
    ];
    let mount = format!("{probe}:/probe:ro");
    // From the browser container's namespace and from Chrome's own, IPv6 and Docker's resolver included.
    let direct =
        [&direct[..], &["tcp", "[2606:4700:4700::1111]:443", "tcp", "[::1]:22", "udp", "127.0.0.11:53"]].concat();
    for chrome in [false, true] {
        let command = [&["/probe"][..], &direct].concat();
        let (ok, text) = in_browser(&id, chrome, &[mount.as_str()], &command);
        assert!(ok, "{text}");
        assert_eq!(text.lines().count(), direct.len() / 2, "{text}");
        for line in text.lines() {
            assert!(!line.ends_with(" ok"), "the browser reached a destination directly: {line}");
        }
    }
    // Through the proxy: the forwarder on loopback is the only way out.
    let proxy = "127.0.0.1:3128";
    let through = [
        "connect",
        proxy,
        "127.0.0.1:443",
        "connect",
        proxy,
        "10.0.0.1:443",
        "connect",
        proxy,
        "169.254.169.254:80",
        "connect",
        proxy,
        "localtest.me:443",
        "connect",
        proxy,
        "127.0.0.1.nip.io:443",
        "connect",
        proxy,
        "[::1]:443",
        "connect",
        proxy,
        "example.com:22",
        "connect",
        proxy,
        "example.com:443",
    ];
    let command = [&["/probe"][..], &through].concat();
    let (ok, text) = in_browser(&id, true, &[mount.as_str()], &command);
    assert!(ok, "{text}");
    let lines: Vec<&str> = text.lines().collect();
    for line in &lines[..lines.len() - 1] {
        assert!(line.contains(" 403 ") || line.contains(" 502 "), "a private or forbidden destination passed: {line}");
    }
    assert!(
        lines.last().unwrap().contains(" 200 "),
        "a public HTTPS destination is admitted: {}",
        lines.last().unwrap()
    );
    admin.stop(&id).await;
}

/// Item 5: the controller's closed set, refused from inside the admin's own identity; and only its uid is served.
#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn the_controller_refuses_everything_outside_its_closed_set() {
    let volume = docker(&[
        "inspect",
        "-f",
        "{{range .Mounts}}{{if eq .Destination \"/run/controller\"}}{{.Name}}{{end}}{{end}}",
        ADMIN_CONTAINER,
    ])
    .1;
    let mount = format!("{}:/run/controller", volume.trim());
    let client = binary("ctl-probe");
    let probe_mount = format!("{client}:/probe:ro");
    let request = |user: &str, line: &str| -> String {
        let options = ["--network", "none", "--user", user, "-v", &mount, "-v", &probe_mount];
        in_probe(&options, &["/probe", "/run/controller/controller.sock", line]).1
    };
    for line in [
        r#"{"op":"exec","session":"0123456789ab","cmd":["sh"]}"#,
        r#"{"op":"create_session","session":"0123456789ab","url":"https://example.com/","devtools":false,"width":1280,"height":800,"binds":["/:/host"]}"#,
        r#"{"op":"create_session","session":"0123456789ab","url":"file:///etc/passwd","devtools":false,"width":1280,"height":800}"#,
        r#"{"op":"create_session","session":"../../etc","url":"https://example.com/","devtools":false,"width":1280,"height":800}"#,
        r#"{"op":"agent_attach","session":"0123456789ab"}"#,
        r#"{"op":"stop_session","session":"0123456789ab","all":true}"#,
        r#"{"op":"create_container","image":"alpine","privileged":true}"#,
    ] {
        let answer = request("10004:10015", line);
        assert!(answer.contains("\"ok\":false"), "the controller accepted {line}: {answer}");
    }
    // Stopping a session the controller does not hold reclaims what carries its label, here nothing.
    let answer = request("10004:10015", r#"{"op":"stop_session","session":"0123456789ab"}"#);
    assert!(answer.contains(r#""reclaimed":{"containers":[],"networks":[],"volumes":[]}"#), "{answer}");
    // Another uid gets no answer at all.
    let answer = request("10001:10015", r#"{"op":"cleanup"}"#);
    assert!(answer.trim() == "closed" || answer.trim().is_empty(), "the controller answered another uid: {answer}");
}

/// Item 6: Chromium's uid cannot reach the privileged socket, and Chromium's socket offers no privileged protocol.
#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn the_privileged_socket_is_out_of_the_browsers_reach() {
    let admin = Admin::login().await;
    let id = session(&admin, false).await;
    let globals = binary("wl-globals");
    let probe_mount = format!("{globals}:/probe:ro");
    let run = |user: &str, groups: &[&str], volume: &str, socket: &str| -> (bool, String) {
        let mount = format!("{INSTANCE}-{id}-{volume}:/run/s:ro");
        let mut args = vec!["run", "--rm", "--network", "none", "--user", user];
        for g in groups {
            args.extend(["--group-add", g]);
        }
        let path = format!("/run/s/{socket}");
        args.extend(["-v", mount.as_str(), "-v", probe_mount.as_str(), PROBE_IMAGE, "/probe", path.as_str()]);
        docker(&args)
    };
    const PRIVILEGED: [&str; 5] = [
        "zwlr_foreign_toplevel_manager_v1",
        "zwlr_screencopy_manager_v1",
        "zwlr_virtual_pointer_manager_v1",
        "zwp_virtual_keyboard_manager_v1",
        "ext_foreign_toplevel_list_v1",
    ];
    // Chromium's uid with Chromium's groups cannot even open the privileged directory.
    let (ok, text) = run("10001:10001", &["10011", "10012", "10016"], "agent", "wayland-0");
    assert!(!ok && text.contains("Permission denied"), "uid A reached the privileged socket: {text}");
    // Chromium's own socket offers none of the privileged globals.
    let (ok, text) = run("10001:10001", &["10011"], "wayland", "wayland-0");
    assert!(ok, "{text}");
    assert!(text.contains("xdg_wm_base"), "{text}");
    for global in PRIVILEGED {
        assert!(!text.contains(global), "Chromium's socket offers {global}");
    }
    // The agent's uid and group see them.
    let (ok, text) = run("10005:10005", &["10013"], "agent", "wayland-0");
    assert!(ok, "{text}");
    for global in PRIVILEGED {
        assert!(text.contains(global), "the agent's socket lacks {global}: {text}");
    }
    admin.stop(&id).await;
}

/// In DevTools mode the agent runs as Chrome's uid in Chrome's PID namespace: both of its long-lived processes, the main
/// process and the driver, are execute-only binaries the kernel marks non-dumpable, so Chrome's uid without capabilities
/// cannot enter their mount namespace or read their descriptors or environment through /proc.
#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn chromes_uid_cannot_reach_into_the_agent_through_proc() {
    let admin = Admin::login().await;
    // The session waits until the agent has listed a window, so the driver runs.
    let id = session(&admin, true).await;
    // `try` prints whether reading one /proc entry worked; the probe's own shell, dumpable, is the control.
    let script = "try() { if $1 /proc/$2/$3 >/dev/null 2>/tmp/e; then echo \"$4 $3 reached\"; \
                    elif grep -q 'Permission denied' /tmp/e; then echo \"$4 $3 denied\"; else echo \"$4 $3 $(cat /tmp/e)\"; fi; }; \
                  try ls $$ root/ control; try cat $$ environ control; \
                  for p in /proc/[0-9]*; do n=$(cat $p/comm 2>/dev/null); \
                    case \"$n\" in agent-idle|cua-driver) ;; *) continue;; esac; \
                    try ls ${p#/proc/} root/ $n; try ls ${p#/proc/} fd/ $n; try cat ${p#/proc/} environ $n; \
                  done";
    let browser = format!("container:{INSTANCE}-{id}-browser");
    let as_chrome = ["--user", "10001:10001", "--cap-drop", "ALL", "--security-opt", "no-new-privileges:true"];
    let (ok, text) =
        in_probe(&[&["--network", "none", "--pid", &browser], &as_chrome[..]].concat(), &["sh", "-c", script]);
    assert!(ok, "{text}");
    let seen: BTreeSet<String> = text.lines().map(str::to_owned).collect();
    let mut expected = BTreeSet::from(["control root/ reached".to_owned(), "control environ reached".to_owned()]);
    for name in ["agent-idle", "cua-driver"] {
        for file in ["root/", "fd/", "environ"] {
            expected.insert(format!("{name} {file} denied"));
        }
    }
    assert_eq!(seen, expected, "Chrome's uid reached into the agent:\n{text}");
    admin.stop(&id).await;
}

/// Item 11: Chromium keeps its sandbox, and kiosk pages cannot open internal schemes.
#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn chromium_keeps_its_sandbox_and_blocks_internal_schemes() {
    let admin = Admin::login().await;
    let id = admin.create("https://fixture.bench/schemes.html").await;
    // Once the page has loaded, its renderer runs.
    admin.wait_title(&id, "Schemes").await;
    // Renderers run in their own PID namespace with seccomp filtering on.
    // Only Chromium's own processes: this script's text would match its own patterns otherwise.
    let script = "for p in /proc/[0-9]*; do c=$(tr '\\0' ' ' < $p/cmdline 2>/dev/null); case \"$c\" in /opt/google/chrome/*) ;; *) continue;; esac; \
                  case \"$c\" in *--no-sandbox*) echo UNSANDBOXED;; esac; case \"$c\" in *--type=renderer*) \
                  echo \"$(grep -E '^Seccomp:' $p/status | tr -d '\\t') $(grep -E '^NSpid:' $p/status | tr '\\t' ' ')\";; esac; done; echo end";
    let browser = format!("container:{INSTANCE}-{id}-browser");
    let (ok, text) = in_probe(&["--network", "none", "--pid", &browser], &["sh", "-c", script]);
    assert!(ok, "{text}");
    let renderers: Vec<&str> = text.lines().filter(|l| l.starts_with("Seccomp")).collect();
    assert!(!renderers.is_empty(), "no renderer found: {text}");
    for r in &renderers {
        assert!(r.starts_with("Seccomp:2"), "a renderer without seccomp filtering: {r}");
        assert!(r.split_whitespace().count() >= 4, "a renderer in the browser's own PID namespace: {r}");
    }
    assert!(!text.contains("UNSANDBOXED"), "a Chromium process runs with --no-sandbox: {text}");
    // The page tries every internal scheme; none opens.
    let (pid, window) = admin.wait_windows(&id, "the schemes page's window", common::first_window).await;
    for link in ["chrome version", "devtools", "local file", "view source", "untrusted"] {
        let script = format!(
            "const app = await cua.getApp({{pid: {pid}, windowId: {window}}}); await app.click({{role: 'link', name: '{link}'}}); \
             await cua.sleep(1500); const w = await cua.listWindows({{pid: {pid}}}); return w.map(x => x.title);"
        );
        let result = admin.agent(&id, "run_script", json!({"script": script})).await.expect("the click runs");
        let text = result.to_string();
        // Either the click did nothing, or Chromium replaced the navigation with its own block page.
        assert!(text.contains("Schemes") || text.contains("about:blank#blocked"), "{link} opened something: {text}");
        for leaked in ["About Version", "root:x:0", "DevTools", "view-source"] {
            assert!(!text.contains(leaked), "{link} opened {leaked}: {text}");
        }
    }
    admin.stop(&id).await;
}

/// What the stealth probe fixture (bench/fixtures/stealth.html) reports, as `name -> value` (JSON text).
async fn probe(admin: &Admin, query: &str) -> std::collections::HashMap<String, String> {
    let id = admin.create(&format!("https://fixture.bench/stealth.html{query}")).await;
    report(admin, &id).await
}

/// The stealth fixture's report in session `id`, which then stops.
async fn report(admin: &Admin, id: &str) -> std::collections::HashMap<String, String> {
    let done = |windows: &[Value]| {
        let title = windows.first()?["title"].as_str()?;
        if title.starts_with("Stealth probe done") { common::first_window(windows) } else { None }
    };
    admin.wait_windows(id, "the stealth probe's report", done).await;
    let text = common::page_text(admin, id).await;
    admin.stop(id).await;
    let start = text.find("webdriver: ").expect("the probe's report");
    // The report is one text node: its last line ends where the node's attributes begin.
    let report = text[start..].split("\" [").next().unwrap_or_default();
    report
        .split('\n')
        .filter_map(|l| l.split_once(": ").map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned())))
        .collect()
}

/// Pages see an ordinary desktop Google Chrome: no automation flag, Google's brands, and WebGL on a real (software)
/// renderer rather than none or SwiftShader.
#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn pages_see_an_ordinary_chrome() {
    let admin = Admin::login().await;
    let seen = probe(&admin, "").await;
    let value = |k: &str| seen.get(k).cloned().unwrap_or_default();
    assert_eq!(value("webdriver"), "false", "{seen:?}");
    // Online, as on a desktop with a network (the browser's own namespace has a link; see browser/launch netns.rs).
    assert_eq!(value("onLine"), "true", "{seen:?}");
    // Colour emoji, and Chinese, Japanese and Korean text, draw with real glyphs.
    assert_eq!(value("emojiColor"), "true", "{seen:?}");
    assert_eq!(value("cjkGlyphs"), "[true,true,true]", "{seen:?}");
    assert!(value("brands").contains("Google Chrome"), "{seen:?}");
    let webgl = value("webgl");
    assert!(webgl.contains("ANGLE") && !webgl.contains("SwiftShader") && webgl != "none", "{seen:?}");
    // Permissions read as in a fresh profile: asked for, not blocked. The microphone stays blocked (see docs/security.md).
    assert_eq!(value("notificationPermission"), "default", "{seen:?}");
    for name in ["notifications", "geolocation", "camera"] {
        assert_eq!(value(&format!("permission.{name}")), "prompt", "{name}: {seen:?}");
    }
    // The deployment's time zone and languages (compose.bench.yaml), the same in pages, workers and requests.
    assert_eq!(value("timezone"), "America/Sao_Paulo", "{seen:?}");
    assert_eq!(value("offsets"), "[180,180]", "{seen:?}");
    assert_eq!(value("languages"), r#"["pt-BR","pt","en-US","en"]"#, "{seen:?}");
    assert_eq!(value("locale"), "pt-BR", "{seen:?}");
    assert_eq!(value("numberLocale"), "pt-BR", "{seen:?}");
    assert!(value("worker").ends_with(r#"["pt-BR","pt","en-US","en"],"pt-BR","America/Sao_Paulo"]"#), "{seen:?}");
    assert!(value("acceptLanguage").starts_with("pt-BR,pt;q=0.9,en-US;q=0.8,en;q=0.7"), "{seen:?}");
}

/// Pages see an ordinary disk's storage quota, while what one site may store stays near the profile's former bound.
#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_site_sees_an_ordinary_quota_but_stores_only_a_bounded_amount() {
    let admin = Admin::login().await;
    let seen = probe(&admin, "?fill").await;
    let value = |k: &str| seen.get(k).cloned().unwrap_or_default();
    let number = |k: &str| value(k).parse::<u64>().unwrap_or_else(|_| panic!("{k}: {seen:?}"));
    assert!(number("storageQuotaMB") >= 10 * 1024, "{seen:?}");
    assert_eq!(value("fillStop"), "QuotaExceededError", "{seen:?}");
    assert!((64..=256).contains(&number("filledMB")), "{seen:?}");
}

/// A person's DevTools shortcuts open no DevTools frontend (Chromium refuses it by policy), even though the launcher's
/// private DevTools pipe may inspect pages for download sizes.
#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn devtools_shortcuts_open_nothing() {
    use display_protocol::keys::{C, F12, I, J, LEFTCTRL, LEFTSHIFT};
    let admin = Admin::login().await;
    let id = admin.create("https://fixture.bench/article.html").await;
    let person = admin.watch(&id, 0).await;
    admin.wait_title(&id, "Static article").await;
    // Chrome's process, by the first listing: while Chrome shows a notice, an unfiltered listing may name its window
    // without a pid (docs/agent-compatibility.md), and a listing scoped by pid still finds it.
    let chrome =
        admin.wait_windows(&id, "Chrome's window", |windows| windows.iter().find_map(|w| w["pid"].as_u64())).await;
    let tree = || async {
        let windows = admin.agent(&id, "list_windows", json!({"pid": chrome})).await.unwrap();
        let mut seen = windows.to_string();
        for w in windows["structuredContent"]["windows"].as_array().into_iter().flatten() {
            let (Some(pid), Some(window)) = (w["pid"].as_u64(), w["window_id"].as_u64()) else { continue };
            let state = admin.agent(&id, "get_window_state", json!({"pid": pid, "window_id": window})).await;
            seen.push_str(&state.map(|s| s.to_string()).unwrap_or_default());
        }
        seen
    };
    let before = tree().await;
    assert!(before.contains("pid=") && before.len() > 1000, "the page's tree was not read: {before}");
    let held = admin.take(&id, &person).await;
    for chord in [&[F12][..], &[LEFTCTRL, LEFTSHIFT, I], &[LEFTCTRL, LEFTSHIFT, J], &[LEFTCTRL, LEFTSHIFT, C]] {
        person.chord(held, chord).await;
        tokio::time::sleep(Duration::from_millis(1500)).await;
    }
    admin.hand_back(&id, &person).await;
    let after = tree().await;
    // The shortcuts reached Chromium, which refused them: its notice (an alert naming DevTools, in the browser's
    // language) is shown, and no panel of the frontend.
    let notice = after
        .split(r#"alert \""#)
        .skip(1)
        .any(|rest| rest.split(r#"\""#).next().unwrap_or_default().contains("DevTools"));
    assert!(notice, "the shortcuts did not reach Chromium: {after}");
    // In the agent's JSON, a tab is a quoted name.
    for frontend in ["devtools://", r#"\"Elements\""#, r#"\"Console\""#, r#"\"Sources\""#, r#"\"Network\""#] {
        assert!(!after.contains(frontend), "a DevTools frontend opened ({frontend}): {after}");
    }
    person.close().await;
    admin.stop(&id).await;
}

/// Item 12 (with the admin's unit tests): logging out closes every live stream at once.
#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn logout_closes_live_streams() {
    let admin = Admin::login().await;
    let id = session(&admin, false).await;
    let viewer = admin.watch(&id, 0).await;
    viewer.wait_lease(display_protocol::LeaseHolder::Agent).await;
    admin.call(reqwest::Method::POST, "/api/logout", None).await.expect("logout");
    viewer.wait_for("the end of the stream at logout", |s| s.closed.then_some(())).await;
    let again = Admin::login().await;
    again.stop(&id).await;
}

/// Starts the test upstream again when dropped, whatever the test reached: the test's later steps and the next run's
/// egresses go through it. No other test uses it, so stopping it reaches no one else.
struct UpstreamBack;

impl Drop for UpstreamBack {
    fn drop(&mut self) {
        let (ok, text) = docker(&["start", UPSTREAM]);
        assert!(ok || std::thread::panicking(), "docker start {UPSTREAM}: {text}");
    }
}

/// The containers, networks and volumes Docker creates while it lives, as `docker events` reports them, so a test can
/// tell the egress checks it caused from what other tests running beside it create. No other test checks an egress.
struct Created {
    events: std::process::Child,
    seen: std::sync::Arc<std::sync::Mutex<Vec<Value>>>,
}

impl Created {
    fn watch() -> Created {
        // From just before it subscribes: Docker replays what it published since, so nothing falls in between.
        let filters =
            ["type=container", "type=network", "type=volume", "event=create"].map(|f| ["--filter", f]).concat();
        let mut events = std::process::Command::new("docker")
            .args([&["events", "--since", &now(), "--format", "{{json .}}"][..], &filters].concat())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("docker events runs");
        let stdout = events.stdout.take().expect("its output");
        let seen = std::sync::Arc::<std::sync::Mutex<Vec<Value>>>::default();
        let into = std::sync::Arc::clone(&seen);
        std::thread::spawn(move || {
            for line in std::io::BufRead::lines(std::io::BufReader::new(stdout)).map_while(Result::ok) {
                into.lock().unwrap().push(serde_json::from_str(&line).expect("an event"));
            }
        });
        Created { events, seen }
    }

    /// What the egress checks created until now left: each one's container, and the networks created meanwhile that
    /// are a check's, that still exist. A check removes both before it answers. At least `checks` must have been seen.
    async fn left_by_checks(&self, checks: usize) -> String {
        // Docker reports its events in order: once a volume created now is seen, so is everything created before it.
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let mark = format!("easi-run-e2e-checks-{}-{now}", std::process::id());
        let (ok, text) = docker(&["volume", "create", &mark]);
        assert!(ok, "docker volume create {mark}: {text}");
        let mut events = Vec::new();
        let marked = common::until!(40, Duration::from_millis(250), {
            events = self.seen.lock().unwrap().clone();
            events.iter().any(|e| e["Type"] == "volume" && e["Actor"]["ID"] == mark.as_str()).then_some(())
        });
        let (removed, text) = docker(&["volume", "rm", &mark]);
        assert!(marked.is_some(), "docker events did not report {mark}");
        assert!(removed, "docker volume rm {mark}: {text}");
        let check = |e: &Value| {
            let labels = &e["Actor"]["Attributes"];
            e["Type"] == "container" && labels["run.easi.instance"] == INSTANCE && labels["run.easi.role"] == "check"
        };
        let seen = events.iter().filter(|e| check(e)).count();
        assert!(seen >= checks, "{seen} egress checks seen, not at least {checks}: {events:?}");
        let id = |e: &Value| e["Actor"]["ID"].as_str().map(str::to_owned);
        let containers: BTreeSet<String> = events.iter().filter(|e| check(e)).filter_map(id).collect();
        let networks: BTreeSet<String> = events.iter().filter(|e| e["Type"] == "network").filter_map(id).collect();
        // Those of them that exist still, and carry a check's labels.
        let instance = format!("label=run.easi.instance={INSTANCE}");
        let filters = ["--filter", &instance, "--filter", "label=run.easi.role=check"];
        let existing = |list: &[&str], of: &BTreeSet<String>| {
            let (ok, text) = docker(&[list, &filters].concat());
            assert!(ok, "docker {list:?}: {text}");
            text.lines().filter(|l| of.contains(*l)).map(str::to_owned).collect::<Vec<_>>()
        };
        let mut left = existing(&["ps", "-aq", "--no-trunc"], &containers);
        left.extend(existing(&["network", "ls", "-q", "--no-trunc"], &networks));
        left.join(" ")
    }
}

impl Drop for Created {
    fn drop(&mut self) {
        let _ = self.events.kill();
        let _ = self.events.wait();
    }
}

/// An egress is added in the dashboard only once its check passed: through it to Cloudflare's trace page, from a
/// network like a session proxy's, which says where it leaves (its country, and the place that suggests); one whose
/// upstream does not answer is never saved, and a session never starts through it. A session through an egress says
/// it is where that egress leaves, and its traffic leaves only through it: names resolve through it (DNS over HTTPS
/// to 1.1.1.1), and the fixture host is asked of it by name. When the upstream stops answering mid-session, fresh
/// names and addresses fail and nothing leaves directly. It runs beside the other tests: it alone uses the test upstream
/// it stops, and checks an egress.
#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn an_egress_is_checked_before_use_and_a_session_leaves_only_through_it() {
    let admin = Admin::login().await;
    let created = Created::watch();
    admin.remove_named("egresses", |name| name.starts_with("e2e-")).await;
    let place = json!({"timezone": "Europe/Berlin", "languages": ["de-DE", "de", "en-US", "en"]});
    let mut berlin = json!({"name": "e2e-berlin", "proxy": UPSTREAM_URL});
    berlin.as_object_mut().unwrap().extend(place.as_object().unwrap().clone());
    let berlin = admin.call(reqwest::Method::POST, "/api/egresses", Some(berlin)).await.expect("berlin passes");
    let country = berlin["country"].as_str().expect("the country its check found").to_owned();
    assert!(country.len() == 2 && berlin["latency_ms"].as_u64().is_some(), "{berlin}");
    assert_eq!(berlin["timezone"], "Europe/Berlin", "a stated place stands");
    // One added without a place takes what its country suggests, as a check of it says.
    let suggested = json!({"name": "e2e-suggested", "proxy": UPSTREAM_URL});
    let suggested = admin.call(reqwest::Method::POST, "/api/egresses", Some(suggested)).await.expect("it passes");
    let path = format!("/api/egresses/{}/check", suggested["id"].as_str().unwrap());
    let checked = admin.call(reqwest::Method::POST, &path, None).await.expect("a check");
    assert_eq!((&checked["ok"], &checked["exit"]["country"]), (&json!(true), &json!(country)), "{checked}");
    assert!(checked["exit"]["ip"].as_str().is_some_and(|ip| ip.parse::<std::net::IpAddr>().is_ok()), "{checked}");
    assert_eq!(suggested["timezone"], checked["suggested"]["timezone"]);
    assert_eq!(suggested["languages"], checked["suggested"]["languages"]);
    // Through an address where nothing listens: refused with the check's reason, never saved, its credential
    // never shown.
    let nowhere = json!({"name": "e2e-nowhere", "proxy": "socks5://user-{session}:test-password@10.250.250.21:1080"});
    let refused = admin.call(reqwest::Method::POST, "/api/egresses", Some(nowhere)).await.unwrap_err();
    assert_eq!(refused.0, 422, "{refused:?}");
    let answer: Value = serde_json::from_str(&refused.1).unwrap();
    assert!(["unreachable", "timeout"].contains(&answer["reason"].as_str().unwrap_or_default()), "{answer}");
    assert!(!refused.1.contains("test-password"));
    let listed = admin.get("/api/egresses").await.unwrap().to_string();
    assert!(listed.contains("e2e-berlin") && !listed.contains("e2e-nowhere"), "{listed}");
    // Four checks so far: of each egress added, and of the one checked.
    assert_eq!(created.left_by_checks(4).await, "", "a check left a container or network");

    // Through it, the browser says it is in Berlin, and the fixture host is asked of the upstream by name.
    let egress = berlin["id"].as_str().unwrap();
    let since = now();
    let id = admin.create_with(json!({"url": "https://fixture.bench/stealth.html", "egress": egress})).await;
    let seen = report(&admin, &id).await;
    let value = |k: &str| seen.get(k).cloned().unwrap_or_default();
    assert_eq!(value("timezone"), "Europe/Berlin", "{seen:?}");
    // Berlin keeps daylight saving time: an hour ahead of UTC in January, two in July.
    assert_eq!(value("offsets"), "[-60,-120]", "{seen:?}");
    assert_eq!(value("languages"), r#"["de-DE","de","en-US","en"]"#, "{seen:?}");
    assert!(value("acceptLanguage").starts_with("de-DE,de;q=0.9"), "{seen:?}");
    let asked = |l: &String| l.contains("\"host\":\"fixture.bench\"") && l.contains("allowed");
    let new = logged_since(UPSTREAM, &since, |lines| lines.iter().any(asked)).await;
    assert!(new.iter().any(asked), "{new:?}");

    // A public site: its name resolves through the upstream, and the connection goes through it by address.
    let since = now();
    let id = admin.create_with(json!({"url": "https://example.com/", "egress": egress})).await;
    let described = admin.get("/api/sessions").await.unwrap().to_string();
    assert!(described.contains(egress), "the session names its egress: {described}");
    let loaded = common::until!(60, Duration::from_millis(500), {
        let windows = admin.agent(&id, "list_windows", json!({})).await.map(|w| w.to_string()).unwrap_or_default();
        windows.contains("Example Domain").then_some(())
    })
    .is_some();
    let proxy = format!("{INSTANCE}-{id}-proxy");
    assert!(loaded, "example.com did not load through the egress: {}", docker(&["logs", &proxy]).1);
    // Held while it runs.
    let held = admin.get("/api/egresses").await.unwrap();
    let entry = held["egresses"].as_array().unwrap().iter().find(|e| e["id"] == egress).unwrap().clone();
    assert_eq!(entry["in_use"], 1, "{entry}");
    let refused = admin.call(reqwest::Method::DELETE, &format!("/api/egresses/{egress}"), None).await.unwrap_err();
    assert!(refused.0 == 409 && refused.1.contains(&id), "{refused:?}");
    // Tunnels are recorded as they close: stopping the session closes them.
    admin.stop(&id).await;
    let resolved = |l: &String| l.contains("\"host\":\"1.1.1.1\"") && l.contains("allowed");
    let connected = |l: &String| l.contains("\"port\":443") && l.contains("allowed") && !l.contains("1.1.1.1");
    let new = logged_since(UPSTREAM, &since, |lines| lines.iter().any(resolved) && lines.iter().any(connected)).await;
    assert!(new.iter().any(resolved), "no DNS over HTTPS: {new:?}");
    assert!(new.iter().any(connected), "the site did not go through the upstream: {new:?}");

    // Another session through it, loaded, before its upstream stops.
    let id = admin.create_with(json!({"url": "https://example.com/", "egress": egress})).await;
    let loaded = common::until!(60, Duration::from_millis(500), {
        let windows = admin.agent(&id, "list_windows", json!({})).await.map(|w| w.to_string()).unwrap_or_default();
        windows.contains("Example Domain").then_some(())
    })
    .is_some();
    let proxy = format!("{INSTANCE}-{id}-proxy");
    assert!(loaded, "example.com did not load through the egress: {}", docker(&["logs", &proxy]).1);
    // The upstream stops answering: a fresh name and an address literal fail, and nothing leaves directly.
    let _back = UpstreamBack;
    let (ok, text) = docker(&["stop", "-t", "1", UPSTREAM]);
    assert!(ok, "docker stop {UPSTREAM}: {text}");
    let stopped = now();
    let stopped_ms = unix_ms_now();
    let (pid, window) = admin.wait_windows(&id, "the page's window", common::first_window).await;
    for url in ["https://example.org/", "https://1.0.0.1/"] {
        let target = json!({"pid": pid, "window_id": window});
        let mut keys = target.clone();
        keys["keys"] = json!(["ctrl", "l"]);
        admin.agent(&id, "hotkey", keys).await.expect("Ctrl+L");
        let mut text = target.clone();
        text["text"] = json!(url);
        admin.agent(&id, "type_text", text).await.expect("the address");
        let mut enter = target;
        enter["key"] = json!("return");
        admin.agent(&id, "press_key", enter).await.expect("Enter");
    }
    let failed = |l: &String| {
        ["upstream_failed", "dns_error", "dns_timeout"].iter().any(|r| l.contains(&format!("\"reason\":\"{r}\"")))
    };
    // Watched for at least six seconds, so a direct connection Chrome makes after the first refusal is seen too.
    let started = std::time::Instant::now();
    let watched = |lines: &[String]| lines.iter().any(failed) && started.elapsed() >= Duration::from_secs(6);
    let lines = logged_since(&proxy, &stopped, watched).await;
    let inspected = docker(&["inspect", &proxy]).1;
    admin.stop(&id).await;
    assert!(lines.iter().any(failed), "{lines:?}");
    // Tunnels are recorded as they close: one the upstream carried before it stopped may close only now. Only those
    // opened since count.
    let opened_since = |l: &&String| {
        let record: Value = serde_json::from_str(l).unwrap_or_default();
        let closed = record["ts"].as_str().and_then(rfc3339_ms).unwrap_or(i64::MAX);
        closed - record["duration_ms"].as_i64().unwrap_or(0) >= stopped_ms
    };
    let allowed =
        lines.iter().filter(|l| l.contains("\"result\":\"allowed\"")).filter(opened_since).collect::<Vec<_>>();
    assert!(allowed.is_empty(), "something left without the upstream: {allowed:?}");
    assert!(!inspected.contains("10.250.250.20:3128"), "the upstream is in the container's configuration");
    drop(_back);
    // With the upstream gone, a start through it is refused before anything of the session exists.
    let (ok, text) = docker(&["stop", "-t", "1", UPSTREAM]);
    assert!(ok, "{text}");
    let back = UpstreamBack;
    // A check now fails, and forgets the pass a session's start would otherwise reuse.
    let checked = admin.call(reqwest::Method::POST, &format!("/api/egresses/{egress}/check"), None).await.unwrap();
    assert!(checked["ok"] == false && checked["reason"].is_string(), "{checked}");
    let refused = admin
        .call(reqwest::Method::POST, "/api/sessions", Some(json!({"url": "https://example.com/", "egress": egress})))
        .await;
    drop(back);
    let refused = refused.expect_err("no session through a stopped upstream");
    assert_eq!(refused.0, 424, "{refused:?}");
    assert!(refused.1.contains("\"reason\""), "{refused:?}");
    // Three more at least: the first start's own (an egress's addition records no pass), the one asked for, and the
    // refused start's own.
    assert_eq!(created.left_by_checks(7).await, "", "a check left a container or network");
    admin.remove_named("egresses", |name| name.starts_with("e2e-")).await;
}

/// The tests that change the admin for everyone: scripts/e2e.sh runs them alone, after every other test.
mod exclusive {
    use super::*;

    /// Turns the bench admin's second step off when dropped, whatever the test reached: every other end-to-end test logs
    /// in with the password alone.
    struct SecondStepOff;

    impl Drop for SecondStepOff {
        fn drop(&mut self) {
            let (ok, text) = docker(&["exec", ADMIN_CONTAINER, "admin", "reset-second-step"]);
            assert!(ok || std::thread::panicking(), "admin reset-second-step: {text}");
        }
    }

    /// RFC 4648 base32, as the admin gives the authenticator app's secret.
    fn base32(text: &str) -> Vec<u8> {
        let (mut bytes, mut buffer, mut bits) = (Vec::new(), 0u32, 0u32);
        for c in text.bytes() {
            let value = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567".iter().position(|&a| a == c).expect("a base32 character");
            buffer = (buffer << 5) | value as u32;
            bits += 5;
            if bits >= 8 {
                bits -= 8;
                bytes.push((buffer >> bits) as u8);
            }
        }
        bytes
    }

    /// The code an authenticator app shows for `secret`, `ahead` 30-second steps from now (RFC 6238, as the app makes it).
    fn app_code(secret: &[u8], ahead: u64) -> String {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY, secret);
        let mac = ring::hmac::sign(&key, &(now / 30 + ahead).to_be_bytes());
        let mac = mac.as_ref();
        let offset = usize::from(mac[19] & 0x0f);
        let value = u32::from_be_bytes(mac[offset..offset + 4].try_into().unwrap()) & 0x7fff_ffff;
        format!("{:06}", value % 1_000_000)
    }

    /// The second step at sign-in, end to end: the authenticator app turned on behind the login, the password then opening
    /// only a pending step, a code from the app signing in once, and turning it off with the password.
    #[tokio::test]
    #[ignore = "needs the bench stack: scripts/e2e.sh"]
    async fn the_second_step_signs_in_with_a_code_and_turns_off_with_the_password() {
        let admin = Admin::login().await;
        let _off = SecondStepOff;
        let drawn = admin.call(reqwest::Method::POST, "/api/security/app", None).await.expect("a secret");
        let secret = base32(drawn["secret"].as_str().unwrap());
        let first = app_code(&secret, 0);
        let confirmed = admin
            .call(reqwest::Method::POST, "/api/security/app/confirm", Some(json!({"code": first})))
            .await
            .expect("the app turned on");
        assert_eq!(confirmed["recovery_codes"].as_array().map(Vec::len), Some(10));

        let password = std::env::var("ADMIN_PASSWORD").unwrap();
        let post = |path: &str, body: Value| {
            admin.http.post(format!("{}{path}", admin.origin)).header("origin", &admin.origin).json(&body).send()
        };
        let answer = post("/api/login", json!({"password": password})).await.unwrap();
        assert!(answer.status().is_success() && answer.headers().get("set-cookie").is_none(), "the password alone");
        let answer: Value = answer.json().await.unwrap();
        let step = answer["second_step"]["step"].as_str().expect("a pending step").to_owned();
        // The confirming code is used; the next one signs in, once.
        let used = post("/api/login/code", json!({"step": step, "code": first})).await.unwrap();
        assert_eq!(used.status(), reqwest::StatusCode::UNAUTHORIZED);
        let code = app_code(&secret, 1);
        let signed_in = post("/api/login/code", json!({"step": step, "code": code})).await.unwrap();
        assert!(signed_in.status().is_success(), "{}", signed_in.status());
        let cookie = common::session_cookie(&signed_in).expect("the login's cookie");
        let csrf = signed_in.json::<Value>().await.unwrap()["csrf"].as_str().unwrap().to_owned();
        let again = post("/api/login/code", json!({"step": step, "code": code})).await.unwrap();
        assert_eq!(again.status(), reqwest::StatusCode::UNAUTHORIZED, "the step and the code are used");

        let second = admin.with_login(cookie, csrf);
        let wrong = second
            .call(reqwest::Method::POST, "/api/security/app/remove", Some(json!({"password": "not the password"})))
            .await;
        assert!(matches!(wrong, Err(common::Refused(403, _))), "{wrong:?}");
        second
            .call(reqwest::Method::POST, "/api/security/app/remove", Some(json!({"password": password})))
            .await
            .expect("the app turned off");
        assert!(Admin::try_login().await.is_some(), "the password alone signs in again");
    }

    /// Item 12: crash cleanup. Restarted with a session alive, the admin removes every labelled session resource and
    /// says what it removed.
    #[tokio::test]
    #[ignore = "needs the bench stack: scripts/e2e.sh"]
    async fn a_restarted_admin_removes_what_the_last_run_left() {
        let admin = Admin::login().await;
        let id = session(&admin, false).await;
        let (ok, _) = docker(&["restart", ADMIN_CONTAINER]);
        assert!(ok);
        let label = format!("label=run.easi.session={id}");
        let leftovers = || -> String {
            let containers = docker(&["ps", "-aq", "--filter", &label]).1;
            let volumes = docker(&["volume", "ls", "-q", "--filter", &label]).1;
            let networks = docker(&["network", "ls", "-q", "--filter", &label]).1;
            format!("{containers}{volumes}{networks}")
        };
        let mut left = String::new();
        for _ in 0..60 {
            tokio::time::sleep(Duration::from_millis(500)).await;
            left = leftovers();
            if left.trim().is_empty() {
                break;
            }
        }
        assert!(left.trim().is_empty(), "the restarted admin left {left}");
        let (_, log) = docker(&["logs", "--since", "2m", ADMIN_CONTAINER]);
        assert!(log.contains(&format!("{INSTANCE}-{id}-browser")), "the admin did not say what it removed");
        // The stack serves again.
        let back = common::until!(60, Duration::from_millis(500), Admin::try_login().await);
        assert!(back.is_some(), "the admin did not come back");
    }
}
