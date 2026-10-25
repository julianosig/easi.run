//! The admin's subcommands, run as the binary: `admin init`, `admin setup`, `admin reset-second-step` and `admin mcp`,
//! which never start the server.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const ORIGIN: &str = "https://localhost:9443";

/// A fresh state directory, removed when dropped.
struct Dir(PathBuf);

impl Dir {
    fn new() -> Dir {
        let dir = std::env::temp_dir().join(format!("admin-cli-{:032x}", rand::random::<u128>()));
        std::fs::create_dir(&dir).unwrap();
        Dir(dir)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The admin binary with `args`, in an environment that holds only `state` as its state directory.
fn command(state: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_admin"));
    command.args(args).env_clear().env("ADMIN_STATE", state);
    command
}

fn admin(state: &Path, args: &[&str], stdin: &[u8]) -> Output {
    let mut child = command(state, args)
        .env("ADMIN_ORIGIN", ORIGIN)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // A refused run may exit before reading its stdin.
    let _ = child.stdin.take().unwrap().write_all(stdin);
    child.wait_with_output().unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn init_sets_the_password_once_from_stdin_and_says_so_on_stderr_only() {
    let dir = Dir::new();
    let file = dir.0.join("password");
    let set = admin(&dir.0, &["init"], b"correct horse battery staple\n");
    assert_eq!(set.status.code(), Some(0), "{}", stderr(&set));
    assert!(set.stdout.is_empty());
    let hash = std::fs::read_to_string(&file).unwrap();
    assert!(hash.starts_with("$argon2id$"), "{hash}");
    assert!(!stderr(&set).contains("correct horse"), "the password is never echoed");
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);

    let again = admin(&dir.0, &["init"], b"another long password\n");
    assert_eq!(again.status.code(), Some(3), "a password was set: {}", stderr(&again));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), hash, "left as it was");

    std::fs::write(&file, "not a hash").unwrap();
    let corrupt = admin(&dir.0, &["init"], b"correct horse battery staple\n");
    assert_eq!(corrupt.status.code(), Some(1), "{}", stderr(&corrupt));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "not a hash");
}

#[test]
fn init_refuses_a_short_password_or_more_than_one_line_and_other_arguments_are_usage_errors() {
    let dir = Dir::new();
    for refused in [&b"short\n"[..], b"correct horse battery\nstaple\n", b"correct horse battery\r\n", b""] {
        let output = admin(&dir.0, &["init"], refused);
        assert_eq!(output.status.code(), Some(1), "{refused:?}: {}", stderr(&output));
    }
    assert!(!dir.0.join("password").exists());
    for args in [&["init", "now"][..], &["setup", "now"], &["reset-second-step", "now"], &["serve"], &["--help"]] {
        assert_eq!(admin(&dir.0, args, b"").status.code(), Some(2), "{args:?}");
    }
}

/// Writes the setup file a running admin would: `token`, expiring `expires` (Unix seconds).
fn setup_file(dir: &Path, token: &str, expires: u64) {
    std::fs::write(dir.join("setup"), format!(r#"{{"token":"{token}","expires":{expires}}}"#)).unwrap();
}

fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
}

#[test]
fn setup_prints_the_running_admin_s_link_until_a_password_is_set() {
    let dir = Dir::new();
    // Ten minutes and a half left: ten whole ones, whatever the run takes up to half a minute.
    setup_file(&dir.0, "abc", unix_now() + 630);
    let link = admin(&dir.0, &["setup"], b"");
    assert_eq!(link.status.code(), Some(0), "{}", stderr(&link));
    assert_eq!(String::from_utf8_lossy(&link.stdout), format!("{ORIGIN}/#setup=abc\n"));
    assert_eq!(stderr(&link), "admin setup: the link sets the password once, within the next 10 minutes\n");

    let set = admin(&dir.0, &["init"], b"correct horse battery staple\n");
    assert_eq!(set.status.code(), Some(0), "{}", stderr(&set));
    assert!(!dir.0.join("setup").exists(), "init removes the setup file");
    let after = admin(&dir.0, &["setup"], b"");
    assert_eq!(after.status.code(), Some(3), "{}", stderr(&after));
    assert!(after.stdout.is_empty());
    assert_eq!(stderr(&after), "admin setup: a password is set already\n");
}

#[test]
fn setup_fails_on_stderr_without_a_setup_it_can_use() {
    let dir = Dir::new();
    std::fs::write(dir.0.join("setup"), "not a setup").unwrap();
    let broken = admin(&dir.0, &["setup"], b"");
    assert_eq!(broken.status.code(), Some(1), "{}", stderr(&broken));
    assert!(broken.stdout.is_empty());
    assert!(stderr(&broken).starts_with("admin setup: ") && stderr(&broken).contains("holds no setup link"));
    let demo =
        command(&dir.0, &["setup"]).env("ADMIN_ORIGIN", ORIGIN).env("ADMIN_DEMO_SECONDS", "60").output().unwrap();
    assert_eq!(demo.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&demo.stderr).contains("the public demo has no password"));
}

#[test]
fn reset_second_step_removes_the_second_step_and_says_so_on_stderr() {
    let dir = Dir::new();
    let file = dir.0.join("second-step.json");
    let off = admin(&dir.0, &["reset-second-step"], b"");
    assert_eq!(off.status.code(), Some(0), "{}", stderr(&off));
    assert!(stderr(&off).contains("was off already"), "{}", stderr(&off));
    // What a running admin writes once the app is on (its secret and the hash of a recovery code).
    std::fs::write(
        &file,
        format!(r#"{{"app":{{"secret":{:?},"last_step":1}},"recovery":[{:?}]}}"#, [7u8; 20], [9u8; 32]),
    )
    .unwrap();
    let reset = admin(&dir.0, &["reset-second-step"], b"");
    assert_eq!(reset.status.code(), Some(0), "{}", stderr(&reset));
    assert!(reset.stdout.is_empty());
    assert_eq!(
        stderr(&reset),
        "admin reset-second-step: the second step is off; sign in with the password alone, then turn it on again under \
         Security\n"
    );
    assert!(!file.exists());
    std::fs::write(&file, "not json").unwrap();
    assert_eq!(admin(&dir.0, &["reset-second-step"], b"").status.code(), Some(0), "whatever the file holds");
    assert!(!file.exists());
    // A directory where the file is cannot be removed: a failure, said on stderr.
    std::fs::create_dir(&file).unwrap();
    let failed = admin(&dir.0, &["reset-second-step"], b"");
    assert_eq!(failed.status.code(), Some(1), "{}", stderr(&failed));
    assert!(stderr(&failed).starts_with("admin reset-second-step: "));
    let demo = command(&dir.0, &["reset-second-step"]).env("ADMIN_DEMO_SECONDS", "60").output().unwrap();
    assert_eq!(demo.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&demo.stderr).contains("the public demo has no second step"));
}

#[test]
fn mcp_without_a_running_admin_fails_on_stderr_and_writes_nothing_to_stdout() {
    let dir = Dir::new();
    let output = admin(&dir.0, &["mcp"], b"{}\n");
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(stderr(&output).starts_with("admin mcp: ") && stderr(&output).contains("mcp.sock"), "{}", stderr(&output));
}

/// A stand-in for the admin's socket: answers each line it reads with `answer: <line>`, and closes once its input
/// ends, or after `lines` lines when given.
fn socket(dir: &Path, lines: Option<usize>) -> std::thread::JoinHandle<Vec<String>> {
    let listener = UnixListener::bind(dir.join("mcp.sock")).unwrap();
    std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut writer = stream.try_clone().unwrap();
        let mut read = Vec::new();
        for line in BufReader::new(stream).lines() {
            let line = line.unwrap();
            writeln!(writer, "answer: {line}").unwrap();
            read.push(line);
            if Some(read.len()) == lines {
                break;
            }
        }
        read
    })
}

#[test]
fn mcp_relays_stdin_to_the_socket_and_the_socket_to_stdout_until_both_end() {
    let dir = Dir::new();
    let server = socket(&dir.0, None);
    let big = "x".repeat(300 * 1024);
    let input = format!("{{\"id\":1}}\n{big}\n{{\"id\":2}}\n");
    let output = admin(&dir.0, &["mcp"], input.as_bytes());
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(output.stderr.is_empty(), "{}", stderr(&output));
    let expected = format!("answer: {{\"id\":1}}\nanswer: {big}\nanswer: {{\"id\":2}}\n");
    assert!(output.stdout == expected.as_bytes(), "every answer, whole and in order");
    assert_eq!(server.join().unwrap(), ["{\"id\":1}", big.as_str(), "{\"id\":2}"]);
}

#[test]
fn mcp_fails_when_the_admin_closes_the_connection_while_stdin_is_open() {
    let dir = Dir::new();
    let server = socket(&dir.0, Some(1));
    let mut child =
        command(&dir.0, &["mcp"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(b"{\"id\":1}\n").unwrap();
    server.join().unwrap();
    let mut out = String::new();
    child.stdout.take().unwrap().read_to_string(&mut out).unwrap();
    let status = child.wait().unwrap();
    drop(stdin);
    assert_eq!(out, "answer: {\"id\":1}\n");
    assert_eq!(status.code(), Some(1));
    let mut err = String::new();
    child.stderr.take().unwrap().read_to_string(&mut err).unwrap();
    assert!(err.contains("closed the connection"), "{err}");
}
