//! `net-probe [tcp HOST:PORT | udp HOST:PORT | connect PROXY HOST:PORT]...`: tries each destination once and prints one
//! `ok` / `refused` / `timeout` / `error` line per destination, each connection and reply bounded by two seconds
//! (a name resolves outside that bound). For `connect`, the proxy is reached within two seconds and its answer, which
//! may wait on the destination, within twelve; the line carries its HTTP status. Run inside the network namespace under
//! test.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs, UdpSocket};
use std::time::Duration;

const BOUND: Duration = Duration::from_secs(2);

fn resolve(target: &str) -> Option<SocketAddr> {
    target.to_socket_addrs().ok()?.next()
}

fn tcp(target: &str) -> String {
    let Some(addr) = resolve(target) else { return "error unresolvable".into() };
    match TcpStream::connect_timeout(&addr, BOUND) {
        Ok(_) => "ok".into(),
        Err(e) if e.kind() == std::io::ErrorKind::TimedOut => "timeout".into(),
        Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => "refused".into(),
        Err(e) => format!("error {e}"),
    }
}

/// Sends a DNS query and waits for any answer.
fn udp(target: &str) -> String {
    let Some(addr) = resolve(target) else { return "error unresolvable".into() };
    let Ok(socket) = UdpSocket::bind("0.0.0.0:0") else { return "error bind".into() };
    let _ = socket.set_read_timeout(Some(BOUND));
    let query = [
        0x12, 0x34, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 7, b'e', b'x', b'a', b'm', b'p', b'l', b'e', 3, b'c', b'o', b'm', 0,
        0, 1, 0, 1,
    ];
    if let Err(e) = socket.send_to(&query, addr) {
        return format!("error {e}");
    }
    let mut buf = [0u8; 512];
    match socket.recv_from(&mut buf) {
        Ok(_) => "ok".into(),
        Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => "timeout".into(),
        Err(e) => format!("error {e}"),
    }
}

fn connect(proxy: &str, target: &str) -> String {
    let Some(addr) = resolve(proxy) else { return "error unresolvable proxy".into() };
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, BOUND) else { return "error proxy unreachable".into() };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(12)));
    let _ = write!(stream, "CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n");
    let mut buf = [0u8; 64];
    match stream.read(&mut buf) {
        Ok(n) => String::from_utf8_lossy(&buf[..n]).lines().next().unwrap_or("").to_owned(),
        Err(e) => format!("error {e}"),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        let line = match args[i].as_str() {
            "tcp" => {
                i += 1;
                format!("tcp {} {}", args[i], tcp(&args[i]))
            }
            "udp" => {
                i += 1;
                format!("udp {} {}", args[i], udp(&args[i]))
            }
            "connect" => {
                i += 2;
                format!("connect {} {}", args[i], connect(&args[i - 1], &args[i]))
            }
            other => format!("unknown {other}"),
        };
        println!("{line}");
        i += 1;
    }
}
