//! `ctl-probe SOCKET LINE`: sends one request line to a Unix socket and prints the first line of the answer, or
//! `closed` when the peer closes (or says nothing within five seconds).

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (Some(path), Some(line)) = (args.get(1), args.get(2)) else {
        eprintln!("usage: ctl-probe SOCKET LINE");
        std::process::exit(2);
    };
    let Ok(mut stream) = UnixStream::connect(path) else {
        println!("closed");
        return;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    if writeln!(stream, "{line}").is_err() {
        println!("closed");
        return;
    }
    let mut answer = String::new();
    match BufReader::new(stream).read_line(&mut answer) {
        Ok(n) if n > 0 => print!("{answer}"),
        _ => println!("closed"),
    }
}
