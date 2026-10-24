//! The proxy given its upstream on standard input: without a valid line in time it exits, also when the input stays
//! open and silent.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn proxy() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_proxy"));
    command
        .env("PROXY_LISTEN", "127.0.0.1:0")
        .env("PROXY_UPSTREAM_STDIN", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

/// Waits up to `limit` for the child to exit and returns its exit code.
fn exit_within(child: &mut std::process::Child, limit: Duration) -> Option<i32> {
    let start = Instant::now();
    while start.elapsed() < limit {
        if let Some(status) = child.try_wait().unwrap() {
            return status.code();
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    let _ = child.wait();
    None
}

#[test]
fn a_silent_open_input_ends_the_proxy() {
    let mut child = proxy().spawn().unwrap();
    // The input stays open and says nothing.
    let _input = child.stdin.take();
    assert_eq!(exit_within(&mut child, Duration::from_secs(10)), Some(2));
}

#[test]
fn a_malformed_upstream_ends_the_proxy_without_repeating_it() {
    for line in
        [&b"{\"scheme\":\"ftp\",\"host\":\"p.example\",\"port\":21,\"password\":\"hunter2\"}\n"[..], b"hunter2\n"]
    {
        let mut child = proxy().spawn().unwrap();
        child.stdin.take().unwrap().write_all(line).unwrap();
        assert_eq!(exit_within(&mut child, Duration::from_secs(10)), Some(2));
        let output = child.wait_with_output().unwrap();
        let said = String::from_utf8_lossy(&output.stderr) + String::from_utf8_lossy(&output.stdout);
        assert!(!said.contains("hunter2"), "{said}");
    }
}

#[test]
fn a_valid_upstream_is_accepted_and_announced() {
    let mut child = proxy().spawn().unwrap();
    child.stdin.take().unwrap().write_all(b"{\"scheme\":\"http\",\"host\":\"127.0.0.1\",\"port\":9}\n").unwrap();
    let mut out = child.stdout.take().unwrap();
    let mut first = [0u8; 14];
    std::io::Read::read_exact(&mut out, &mut first).unwrap();
    assert_eq!(&first, b"{\"ready\":true}");
    let _ = child.kill();
    let _ = child.wait();
}
