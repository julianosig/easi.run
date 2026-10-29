//! Runs the real `display` binary on private socket paths and observes the process from outside.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// The `display` binary: `DISPLAY_BIN` when set, otherwise built once with the test's own profile, into its own target
/// directory (named in full: a relative `CARGO_TARGET_DIR` would move with the build's directory).
pub fn display_bin() -> &'static Path {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| {
        if let Some(path) = std::env::var_os("DISPLAY_BIN").filter(|p| !p.is_empty()) {
            return PathBuf::from(path);
        }
        // The test executable lives in `<target>/<profile>/deps/`, or `<target>/<triple>/<profile>/deps/` when the
        // build names the triple it builds for: then this build names it too.
        let exe = std::env::current_exe().expect("the test executable");
        let profile_dir = exe.parent().and_then(Path::parent).expect("<target>/<profile>").to_path_buf();
        let profile = match profile_dir.file_name().and_then(|n| n.to_str()) {
            Some("debug") | None => "dev".to_string(),
            Some(name) => name.to_string(),
        };
        let mut target_dir = profile_dir.parent().expect("<target>");
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let mut build = Command::new(cargo);
        build.args(["build", "--quiet", "-p", "display", "--profile", &profile]);
        if target_dir.file_name().is_some_and(|name| name == env!("TARGET")) {
            build.args(["--target", env!("TARGET")]);
            target_dir = target_dir.parent().expect("<target>");
        }
        let status = build
            .arg("--target-dir")
            .arg(target_dir)
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .status()
            .expect("run cargo build -p display");
        assert!(status.success(), "cargo build -p display failed");
        let bin = profile_dir.join("display");
        assert!(bin.exists(), "{} was not built", bin.display());
        bin
    })
}

/// Raises this process's soft descriptor limit to its hard one, once. The hostile clients keep thousands of descriptors
/// in flight on Unix sockets, and the kernel refuses a send once a user's in-flight descriptors pass the sender's soft
/// limit, which is often 1024. The compositor under test sets its own limit, the one it runs with in a session.
fn raise_descriptor_limit() {
    static RAISED: OnceLock<()> = OnceLock::new();
    RAISED.get_or_init(|| {
        use rustix::process::{Resource, Rlimit, getrlimit, setrlimit};
        let hard = getrlimit(Resource::Nofile).maximum;
        setrlimit(Resource::Nofile, Rlimit { current: hard, maximum: hard }).expect("raise the soft descriptor limit");
    });
}

/// The output every test runs with unless it says otherwise, as width and height.
pub const OUTPUT: (i32, i32) = (320, 240);

/// How a test configures the compositor.
#[derive(Clone, Debug)]
pub struct Options {
    /// The output's width and height.
    pub output: (i32, i32),
    pub max_fps: u32,
    /// The shortest time between two compositions, in milliseconds.
    pub compose_gap_ms: u32,
    /// The only uid the browser socket accepts.
    pub browser_uid: u32,
    /// The only uid the agent and injection sockets accept, when they are configured.
    pub agent_uid: Option<u32>,
    /// Whether the viewer stream socket is configured, for this process's uid as the admin application's.
    pub stream: bool,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            output: OUTPUT,
            max_fps: 60,
            compose_gap_ms: 4,
            browser_uid: current_uid(),
            agent_uid: None,
            stream: false,
        }
    }
}

pub fn current_uid() -> u32 {
    rustix::process::getuid().as_raw()
}

/// One running compositor, killed and cleaned up on drop.
pub struct Compositor {
    child: Child,
    dir: PathBuf,
    pub browser_socket: PathBuf,
    pub agent_socket: Option<PathBuf>,
    pub stream_socket: Option<PathBuf>,
}

impl Compositor {
    pub fn start() -> Compositor {
        Compositor::start_with(Options::default())
    }

    /// One with the agent and injection sockets, for this process's uid.
    pub fn start_with_agent() -> Compositor {
        Compositor::start_with(Options { agent_uid: Some(current_uid()), ..Options::default() })
    }

    pub fn start_with(options: Options) -> Compositor {
        raise_descriptor_limit();
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        // Short paths: a Unix socket path holds at most 107 bytes.
        let dir =
            std::env::temp_dir().join(format!("dh-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create the socket directory");
        let browser_socket = dir.join("browser");
        let log = File::create(dir.join("log")).expect("create the compositor log");
        let mut command = Command::new(display_bin());
        command
            .env_clear()
            .env("RUST_LOG", "info")
            .env("DISPLAY_OUTPUT", format!("{}x{}", options.output.0, options.output.1))
            .env("DISPLAY_MAX_FPS", options.max_fps.to_string())
            .env("DISPLAY_COMPOSE_GAP_MS", options.compose_gap_ms.to_string())
            .env("DISPLAY_BROWSER_SOCKET", &browser_socket)
            .env("DISPLAY_BROWSER_UID", options.browser_uid.to_string())
            .stdin(Stdio::null())
            .stdout(log.try_clone().expect("clone the log"))
            .stderr(log);
        // A coverage run (scripts/coverage.sh) names where the compositor writes its counters.
        if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
            command.env("LLVM_PROFILE_FILE", profile);
        }
        let agent_socket = options.agent_uid.map(|uid| {
            let socket = dir.join("agent");
            command
                .env("DISPLAY_AGENT_SOCKET", &socket)
                .env("DISPLAY_INJECT_SOCKET", dir.join("inject"))
                .env("DISPLAY_AGENT_UID", uid.to_string());
            socket
        });
        let stream_socket = options.stream.then(|| {
            let socket = dir.join("stream");
            command.env("DISPLAY_STREAM_SOCKET", &socket).env("DISPLAY_ADMIN_UID", current_uid().to_string());
            socket
        });
        let child = command.spawn().expect("start the display binary");
        let mut compositor = Compositor { child, dir, browser_socket, agent_socket, stream_socket };
        compositor.wait_ready();
        compositor
    }

    fn wait_ready(&mut self) {
        let mut sockets = vec![self.browser_socket.clone()];
        // The injection socket comes after the agent's: both, before a test connects to either.
        sockets.extend(self.agent_socket.iter().flat_map(|agent| [agent.clone(), agent.with_file_name("inject")]));
        sockets.extend(self.stream_socket.clone());
        let deadline = Instant::now() + Duration::from_secs(10);
        while !sockets.iter().all(|s| s.exists()) {
            self.assert_alive();
            assert!(Instant::now() < deadline, "the compositor did not create its sockets\n{}", self.log());
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Panics with the compositor's log when the process has exited.
    pub fn assert_alive(&mut self) {
        if let Some(status) = self.child.try_wait().expect("poll the compositor") {
            panic!("the compositor exited ({status})\n{}", self.log());
        }
    }

    pub fn log(&self) -> String {
        std::fs::read_to_string(self.dir.join("log")).unwrap_or_default()
    }

    /// Resident set size, in KiB.
    pub fn rss_kib(&self) -> u64 {
        let status = std::fs::read_to_string(format!("/proc/{}/status", self.pid())).expect("read /proc/<pid>/status");
        status
            .lines()
            .find_map(|line| line.strip_prefix("VmRSS:"))
            .and_then(|v| v.trim().trim_end_matches("kB").trim().parse().ok())
            .expect("VmRSS in /proc/<pid>/status")
    }

    /// Open file descriptors.
    pub fn fd_count(&self) -> usize {
        std::fs::read_dir(format!("/proc/{}/fd", self.pid())).expect("read /proc/<pid>/fd").count()
    }

    /// Open file descriptors on `path`, such as `/dev/null`: unlike [`Compositor::fd_count`], blind to the files and
    /// pipes it opens for a moment while it starts.
    pub fn fds_on(&self, path: &str) -> usize {
        std::fs::read_dir(format!("/proc/{}/fd", self.pid()))
            .expect("read /proc/<pid>/fd")
            .filter_map(Result::ok)
            .filter(|fd| std::fs::read_link(fd.path()).is_ok_and(|target| target.as_os_str() == path))
            .count()
    }
}

impl Drop for Compositor {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
