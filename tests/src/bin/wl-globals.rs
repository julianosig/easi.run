//! `wl-globals SOCKET`: prints the interface of every global the compositor offers on SOCKET, one per line, or exits 1
//! when the connection is refused.

use wayland_client::Connection;
use wayland_client::globals::registry_queue_init;

fn main() {
    let path = std::env::args().nth(1).expect("a socket path");
    let stream = match std::os::unix::net::UnixStream::connect(&path) {
        Ok(stream) => stream,
        Err(e) => {
            eprintln!("connect {path}: {e}");
            std::process::exit(1);
        }
    };
    let connection = Connection::from_socket(stream).expect("a Wayland connection");
    let Ok((globals, _queue)) = registry_queue_init::<display_tests::wayland::App>(&connection) else {
        eprintln!("the compositor refused the connection");
        std::process::exit(1);
    };
    let mut names: Vec<String> = globals.contents().clone_list().into_iter().map(|g| g.interface).collect();
    names.sort();
    names.dedup();
    for name in names {
        println!("{name}");
    }
}
