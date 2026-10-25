//! Against the real Docker Engine, ignored by default (scripts/e2e.sh runs it): the controller creates a session from
//! small digest-pinned stand-in images, relays and terminates the agent exec, rolls back a failed creation, counts a
//! crashed run's orphans toward the session limit and reclaims them, and refuses everything outside its closed set.
//!
//! Its controllers run as an instance of their own (`easirun-test`), so the default instance's live sessions, such as
//! the bench stack's, are never touched. Every resource this test creates carries the `run.easi.session` label, and its
//! final `cleanup` removes every labelled resource of that instance, so run it where no other run of it is live. A run
//! that fails removes them too, through [`Residue`].

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const BUSYBOX: &str = "busybox@sha256:dc2d74b28e4cf8984fa52af1f39bc7c3d9c73760b41a74d629f5d11b1ab28616";
const LABEL: &str = "run.easi.session";
const INSTANCE: &str = "run.easi.instance";
/// The test controllers' `CONTROLLER_INSTANCE`.
const NAME: &str = "easirun-test";

fn docker(args: &[&str]) -> String {
    let out = Command::new("docker").args(args).output().expect("docker CLI");
    assert!(out.status.success(), "docker {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}

fn inspect(kind: &str, name: &str) -> Value {
    let text = docker(&[kind, "inspect", name]);
    serde_json::from_str::<Value>(&text).unwrap()[0].clone()
}

/// The names of the containers, networks or volumes (`kind`) that `filter` selects.
fn names(kind: &str, filter: &str) -> Vec<String> {
    let args = match kind {
        "container" => ["ps", "-a", "--filter", filter, "--format", "{{.Names}}"],
        _ => [kind, "ls", "--filter", filter, "--format", "{{.Name}}"],
    };
    docker(&args).lines().map(str::to_owned).collect()
}

fn labelled(kind: &str, session: &str) -> Vec<String> {
    names(kind, &format!("label={LABEL}={session}"))
}

/// Resources of this test's instance that another run left.
fn foreign() -> Vec<String> {
    let filter = format!("label={INSTANCE}={NAME}");
    ["container", "network", "volume"].iter().flat_map(|kind| names(kind, &filter)).collect()
}

/// Removes every resource of this test's instance and the neighbour volume when the test ends, passed or failed, so a
/// failed run leaves nothing that would refuse the next one.
struct Residue {
    neighbour: String,
}

impl Drop for Residue {
    fn drop(&mut self) {
        // Nothing here may panic: it also runs while a failed assertion unwinds.
        let filter = format!("label={INSTANCE}={NAME}");
        let ids = |args: &[&str]| -> Vec<String> {
            Command::new("docker")
                .args(args)
                .output()
                .map(|out| String::from_utf8_lossy(&out.stdout).lines().map(str::to_owned).collect())
                .unwrap_or_default()
        };
        let remove = |args: &[&str], ids: Vec<String>| {
            if !ids.is_empty() {
                let _ = Command::new("docker").args(args).args(ids).output();
            }
        };
        remove(&["rm", "-f", "-v"], ids(&["ps", "-aq", "--filter", &filter]));
        remove(&["network", "rm"], ids(&["network", "ls", "-q", "--filter", &filter]));
        let mut volumes = ids(&["volume", "ls", "-q", "--filter", &filter]);
        volumes.push(self.neighbour.clone());
        remove(&["volume", "rm", "-f"], volumes);
    }
}

fn all_labelled(session: &str) -> Vec<String> {
    ["container", "network", "volume"].iter().flat_map(|kind| labelled(kind, session)).collect()
}

/// Builds a stand-in image and returns its image id (`sha256:`), which every image store gives a local build (a
/// repository digest exists only in the containerd store).
fn standin(dir: &Path, name: &str, agent: bool) -> String {
    let context = dir.join(name);
    std::fs::create_dir_all(&context).unwrap();
    let mut dockerfile = format!("FROM {BUSYBOX}\n");
    if agent {
        std::fs::write(context.join("agent-mcp"), "#!/bin/sh\nexec cat\n").unwrap();
        dockerfile.push_str("COPY --chmod=0755 agent-mcp /usr/local/bin/agent-mcp\n");
    }
    dockerfile.push_str("CMD [\"sleep\", \"3600\"]\n");
    std::fs::write(context.join("Dockerfile"), dockerfile).unwrap();
    docker(&["build", "-q", "-t", &format!("{name}:test"), context.to_str().unwrap()]);
    docker(&["image", "inspect", "-f", "{{.Id}}", &format!("{name}:test")]).trim().to_owned()
}

struct Env {
    dir: PathBuf,
    images: [String; 4],
    seccomp: PathBuf,
    streams: String,
    uid: String,
}

struct Running {
    child: Child,
    socket: PathBuf,
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Env {
    fn controller(&self, tag: &str, admin_uid: &str, agent_image: Option<&str>, envs: &[(&str, &str)]) -> Running {
        let dir = self.dir.join(tag);
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("controller.sock");
        let [display, browser, agent, proxy] = &self.images;
        let child = Command::new(env!("CARGO_BIN_EXE_controller"))
            .env("CONTROLLER_SOCKET", &socket)
            .env("CONTROLLER_INSTANCE", NAME)
            .env("CONTROLLER_ADMIN_UID", admin_uid)
            .env("IMAGE_DISPLAY", display)
            .env("IMAGE_BROWSER", browser)
            .env("IMAGE_AGENT", agent_image.unwrap_or(agent))
            .env("IMAGE_PROXY", proxy)
            .env("CONTROLLER_BROWSER_SECCOMP", &self.seccomp)
            .env("STREAMS_VOLUME", &self.streams)
            .envs(envs.iter().copied())
            .stdout(Stdio::null())
            .spawn()
            .expect("controller");
        let deadline = Instant::now() + Duration::from_secs(20);
        while UnixStream::connect(&socket).is_err() {
            assert!(Instant::now() < deadline, "the controller did not listen");
            std::thread::sleep(Duration::from_millis(50));
        }
        Running { child, socket }
    }

    fn session_dir(&self, session: &str) {
        let mount = format!("{}:/v", self.streams);
        docker(&[
            "run",
            "--rm",
            "--label",
            &format!("{LABEL}=test-helper"),
            "-v",
            &mount,
            BUSYBOX,
            "mkdir",
            "-p",
            &format!("/v/{session}"),
        ]);
    }
}

struct Client {
    reader: BufReader<UnixStream>,
}

impl Client {
    fn new(running: &Running) -> Self {
        let stream = UnixStream::connect(&running.socket).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
        Self { reader: BufReader::new(stream) }
    }

    fn send_raw(&mut self, bytes: &[u8]) -> Option<Value> {
        self.reader.get_mut().write_all(bytes).unwrap();
        let mut line = String::new();
        match self.reader.read_line(&mut line) {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(serde_json::from_str(&line).unwrap()),
        }
    }

    fn call(&mut self, request: Value) -> Value {
        let mut bytes = request.to_string().into_bytes();
        bytes.push(b'\n');
        self.send_raw(&bytes).expect("a response line")
    }

    /// Writes `line` to an attached exec relay, whose stand-in agent echoes it back.
    fn echoes(&mut self, line: &str) {
        self.reader.get_mut().write_all(line.as_bytes()).unwrap();
        let mut echoed = String::new();
        self.reader.read_line(&mut echoed).unwrap();
        assert_eq!(echoed, line);
    }
}

fn create(session: &str, devtools: bool) -> Value {
    json!({"op": "create_session", "session": session, "url": "https://example.com/", "devtools": devtools,
        "width": 1280, "height": 800, "timezone": "UTC", "languages": ["en-US"]})
}

fn assert_hardened(name: &str, user: &str) -> Value {
    let c = inspect("container", name);
    let host = &c["HostConfig"];
    assert_eq!(c["State"]["Running"], true, "{name}");
    assert_eq!(c["Config"]["User"], user, "{name}");
    assert_eq!(host["Privileged"], false, "{name}");
    assert_eq!(host["ReadonlyRootfs"], true, "{name}");
    // Docker's init is the first process of each container but the agent, which runs in the browser's PID namespace.
    assert_eq!(host["Init"].as_bool().unwrap_or(false), !name.ends_with("-agent"), "{name}");
    assert_eq!(host["CapDrop"], json!(["ALL"]), "{name}");
    assert!(host["CapAdd"].is_null(), "{name}");
    assert!(host["Binds"].is_null(), "{name}");
    assert!(host["Devices"].is_null() || host["Devices"] == json!([]), "{name}");
    assert!(host["SecurityOpt"].as_array().unwrap().contains(&json!("no-new-privileges:true")), "{name}");
    // The limits the engine kept: CPU, memory with no swap beyond it, and processes.
    assert!(host["CpuPeriod"].as_i64().unwrap() > 0 && host["CpuQuota"].as_i64().unwrap() > 0, "{name}");
    assert!(host["Memory"].as_i64().unwrap() > 0, "{name}");
    assert_eq!(host["MemorySwap"], host["Memory"], "{name}");
    assert!(host["PidsLimit"].as_i64().unwrap() > 0, "{name}");
    for mount in c["Mounts"].as_array().unwrap() {
        assert_eq!(mount["Type"], "volume", "{name}");
    }
    c
}

#[test]
#[ignore = "needs Docker: scripts/e2e.sh"]
fn sessions_against_the_engine() {
    // `cleanup` removes this instance's labelled resources: never run beside another live session of it.
    let others = foreign();
    assert!(
        others.is_empty(),
        "labelled resources of another run exist, refusing to clean them up: {others:?}; once no run of this test is \
         live, remove them with: docker rm -f $(docker ps -aq --filter label={INSTANCE}={NAME}); docker network rm \
         $(docker network ls -q --filter label={INSTANCE}={NAME}); docker volume rm $(docker volume ls -q --filter \
         label={INSTANCE}={NAME})"
    );
    let neighbour = format!("controller-test-neighbour-{}", std::process::id());
    let _residue = Residue { neighbour: neighbour.clone() };
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("controller-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let idle = standin(&dir, "easirun-standin-idle", false);
    let agent = standin(&dir, "easirun-standin-agent", true);
    let seccomp = dir.join("seccomp.json");
    std::fs::write(&seccomp, r#"{"defaultAction": "SCMP_ACT_ALLOW", "syscalls": []}"#).unwrap();
    let streams = format!("controller-test-streams-{}", std::process::id());
    docker(&[
        "volume",
        "create",
        "--label",
        &format!("{INSTANCE}={NAME}"),
        "--label",
        &format!("{LABEL}=test-streams"),
        &streams,
    ]);
    // Another installation's resource on the same engine, which nothing here may touch.
    docker(&[
        "volume",
        "create",
        "--label",
        &format!("{INSTANCE}=other"),
        "--label",
        &format!("{LABEL}=0123456789ab"),
        &neighbour,
    ]);
    let uid = String::from_utf8(Command::new("id").arg("-u").output().unwrap().stdout).unwrap().trim().to_owned();
    // The proxy comes as a plain local tag, which the controller resolves once to its image id.
    let proxy_id = docker(&["image", "inspect", "-f", "{{.Id}}", "easirun-standin-idle:test"]).trim().to_owned();
    let env =
        Env { dir, images: [idle.clone(), idle, agent, "easirun-standin-idle:test".to_owned()], seccomp, streams, uid };

    let first = env.controller("first", &env.uid, None, &[]);
    let mut c = Client::new(&first);

    // The closed set: a request outside it, and one whose values fail validation, are answered as refused (request.rs
    // tests which requests those are), and unknown sessions are unknown.
    for request in [
        json!({"op": "exec", "session": "0123456789ab", "cmd": ["sh"]}),
        json!({"op": "create_session", "session": "0123456789ab", "url": "file:///etc/passwd", "devtools": false,
            "width": 1280, "height": 800}),
    ] {
        let answer = c.call(request.clone());
        assert_eq!(answer["ok"], false, "{request}");
        assert!(answer["error"].as_str().unwrap().starts_with("refused"), "{request}: {answer}");
    }
    for op in ["inspect_session", "agent_attach", "agent_terminate", "agent_restart"] {
        let answer = c.call(json!({"op": op, "session": "0123456789ab"}));
        assert_eq!(answer, json!({"ok": false, "error": "unknown session"}), "{op}");
    }
    // Stopping a session that left nothing behind succeeds: there is nothing to stop.
    let nothing = json!({"containers": [], "networks": [], "volumes": []});
    let answer = c.call(json!({"op": "stop_session", "session": "0123456789ab"}));
    assert_eq!(answer, json!({"ok": true, "reclaimed": nothing}));
    let mut long = vec![b'x'; 64 * 1024 + 1];
    long.push(b'\n');
    let answer = Client::new(&first).send_raw(&long).expect("a refusal");
    assert_eq!(answer["error"], "request too long");

    // Another uid gets no answer at all. (This client shares the controller's PID namespace; across containers
    // SO_PEERCRED reports pid 0, which the controller's tokio-based check tolerates by design.)
    let strict = env.controller("strict", "10004", None, &[]);
    assert!(Client::new(&strict).send_raw(b"{\"op\":\"cleanup\"}\n").is_none());
    drop(strict);

    // A session from the templates.
    let a = "a0a0a0a0a0a0";
    let b = "b0b0b0b0b0b0";
    env.session_dir(a);
    env.session_dir(b);
    let answer = c.call(create(a, false));
    assert_eq!(answer["ok"], true, "{answer}");
    assert_eq!(c.call(create(a, false))["error"], "the session exists");
    let display = assert_hardened(&format!("{NAME}-{a}-display"), "10002:10002");
    assert_eq!(display["HostConfig"]["NetworkMode"], "none");
    let stream_mount = display["HostConfig"]["Mounts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["Source"] == env.streams.as_str())
        .unwrap()
        .clone();
    assert_eq!(stream_mount["VolumeOptions"]["Subpath"], a);
    let proxy = assert_hardened(&format!("{NAME}-{a}-proxy"), "10003:10003");
    assert_eq!(proxy["HostConfig"]["Sysctls"]["net.ipv4.ip_forward"], "0");
    assert_eq!(proxy["Config"]["Image"], proxy_id.as_str(), "the tag was resolved to the image id");
    let browser = assert_hardened(&format!("{NAME}-{a}-browser"), "10001:10001");
    assert_eq!(browser["HostConfig"]["NetworkMode"], "none");
    assert!(
        browser["HostConfig"]["SecurityOpt"]
            .as_array()
            .unwrap()
            .iter()
            .any(|o| o.as_str().unwrap().starts_with("seccomp="))
    );
    let browser_id = browser["Id"].as_str().unwrap().to_owned();
    let agent_c = assert_hardened(&format!("{NAME}-{a}-agent"), "10005:10005");
    assert_eq!(agent_c["HostConfig"]["PidMode"], format!("container:{browser_id}"));
    assert_eq!(agent_c["HostConfig"]["NetworkMode"], "none");
    let egress = inspect("network", &format!("{NAME}-{a}-egress"));
    assert_eq!(egress["EnableIPv6"], false);
    // The host interface carries the prefix the documented host rule drops (docs/security.md).
    assert_eq!(egress["Options"], json!({"com.docker.network.bridge.name": format!("er-{a}")}));
    assert_eq!(inspect("volume", &format!("{NAME}-{a}-bus"))["Options"]["type"], "tmpfs");
    let mut names = all_labelled(a);
    names.sort();
    assert_eq!(names.len(), 9, "{names:?}");
    // The browser joins no network; it reaches the proxy over the socket volume the two share.
    let networks: Vec<&String> = browser["NetworkSettings"]["Networks"].as_object().unwrap().keys().collect();
    assert_eq!(networks, ["none"]);
    let mounts_proxy =
        |c: &Value| c["Mounts"].as_array().unwrap().iter().any(|m| m["Name"] == format!("{NAME}-{a}-proxy"));
    assert!(mounts_proxy(&browser) && mounts_proxy(&proxy));

    // DevTools mode: the agent runs as the browser's uid in the browser's network namespace.
    assert_eq!(c.call(create(b, true))["ok"], true);
    let browser_b = inspect("container", &format!("{NAME}-{b}-browser"))["Id"].as_str().unwrap().to_owned();
    let agent_b = assert_hardened(&format!("{NAME}-{b}-agent"), "10001:10001");
    assert_eq!(agent_b["HostConfig"]["NetworkMode"], format!("container:{browser_b}"));

    let state = c.call(json!({"op": "inspect_session", "session": a}));
    assert_eq!(state["ok"], true, "{state}");
    for role in ["display", "proxy", "browser", "agent"] {
        assert_eq!(state["containers"][role]["running"], true, "{role}");
        assert!(state["containers"][role]["memory_bytes"].is_u64(), "{role}: {state}");
        assert!(state["containers"][role]["cpu_percent"].is_number(), "{role}: {state}");
    }

    // The one exec, relayed as raw bytes, then terminated with proof and restarted.
    let mut relay = Client::new(&first);
    let answer = relay.call(json!({"op": "agent_attach", "session": a}));
    assert_eq!(answer["ok"], true, "{answer}");
    assert!(answer["exec"].as_str().unwrap().bytes().all(|b| b.is_ascii_hexdigit()), "{answer}");
    relay.echoes("{\"jsonrpc\":\"2.0\"}\n");
    assert!(c.call(json!({"op": "agent_attach", "session": a}))["error"].as_str().unwrap().contains("already"));
    // The agent shares the browser's PID namespace, so the browser sees the exec's process until it is gone.
    let processes = || docker(&["exec", &format!("{NAME}-{a}-browser"), "ps", "-o", "user,args"]);
    assert!(processes().lines().any(|l| l.trim_end().ends_with("cat") && l.starts_with("10005")), "{}", processes());
    let answer = c.call(json!({"op": "agent_terminate", "session": a}));
    assert_eq!(answer, json!({"ok": true, "exited": true, "exit_code": 137}));
    let mut rest = Vec::new();
    let _ = relay.reader.read_to_end(&mut rest);
    assert!(rest.is_empty(), "the relay closes after the exec dies");
    assert_eq!(inspect("container", &format!("{NAME}-{a}-agent"))["State"]["Running"], false);
    assert!(!processes().lines().any(|l| l.trim_end().ends_with("cat")), "{}", processes());
    let answer = c.call(json!({"op": "agent_restart", "session": a}));
    assert_eq!(answer["ok"], true, "{answer}");
    assert_eq!(inspect("container", &format!("{NAME}-{a}-agent"))["State"]["Running"], true);
    let mut relay = Client::new(&first);
    assert_eq!(relay.call(json!({"op": "agent_attach", "session": a}))["ok"], true);
    relay.echoes("again\n");
    drop(relay);

    // stop_session removes everything of that session.
    assert_eq!(c.call(json!({"op": "stop_session", "session": b})), json!({"ok": true}));
    assert!(all_labelled(b).is_empty());
    assert_eq!(c.call(json!({"op": "inspect_session", "session": b}))["error"], "unknown session");

    // A creation that fails half-way (the agent image does not exist) is rolled back entirely.
    let missing = format!("easirun-standin-missing@sha256:{}", "0".repeat(64));
    let failing = env.controller("failing", &env.uid, Some(&missing), &[]);
    let r = "c0c0c0c0c0c0";
    env.session_dir(r);
    let answer = Client::new(&failing).call(create(r, false));
    assert_eq!(answer["ok"], false);
    assert!(answer["error"].as_str().unwrap().contains("rolled back"), "{answer}");
    assert!(all_labelled(r).is_empty(), "{:?}", all_labelled(r));
    drop(failing);

    // A crashed controller leaves session a behind. The next run does not hold it, and stopping it reclaims what
    // carries its label.
    drop(c);
    drop(first);
    // Until then it counts toward the session limit: a run allowed one session has none to spare.
    let capped = env.controller("capped", &env.uid, None, &[("CONTROLLER_MAX_SESSIONS", "1")]);
    let answer = Client::new(&capped).call(create("e0e0e0e0e0e0", false));
    assert!(answer["error"].as_str().unwrap().contains("maximum of 1 sessions"), "{answer}");
    drop(capped);
    let second = env.controller("second", &env.uid, None, &[]);
    let mut c = Client::new(&second);
    let answer = c.call(json!({"op": "stop_session", "session": a}));
    assert_eq!(answer["ok"], true, "{answer}");
    let all_of = |removed: &Value, s: &str| {
        for role in ["display", "proxy", "browser", "agent"] {
            assert!(
                removed["containers"].as_array().unwrap().contains(&json!(format!("{NAME}-{s}-{role}"))),
                "{removed}"
            );
        }
        assert!(removed["networks"].as_array().unwrap().contains(&json!(format!("{NAME}-{s}-egress"))), "{removed}");
        for name in ["wayland", "agent", "bus", "proxy"] {
            assert!(removed["volumes"].as_array().unwrap().contains(&json!(format!("{NAME}-{s}-{name}"))), "{removed}");
        }
    };
    all_of(&answer["reclaimed"], a);
    assert!(all_labelled(a).is_empty());
    // cleanup removes this instance's labelled resources, a held session's included.
    let d = "d0d0d0d0d0d0";
    env.session_dir(d);
    assert_eq!(c.call(create(d, false))["ok"], true);
    let answer = c.call(json!({"op": "cleanup"}));
    assert_eq!(answer["ok"], true, "{answer}");
    all_of(&answer["removed"], d);
    assert!(all_labelled(d).is_empty());
    assert!(all_labelled("test-streams").is_empty(), "cleanup removes this instance's labelled volumes");
    assert_eq!(
        labelled("volume", "0123456789ab"),
        std::slice::from_ref(&neighbour),
        "cleanup left another instance alone"
    );
    docker(&["volume", "rm", &neighbour]);
    let again = c.call(json!({"op": "cleanup"}));
    assert_eq!(again, json!({"ok": true, "removed": {"containers": [], "networks": [], "volumes": []}}));
    drop(second);
    let _ = std::fs::remove_dir_all(&env.dir);
}
