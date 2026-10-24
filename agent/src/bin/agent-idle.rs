//! The agent container's main process: it waits, so the controller can execute the driver in the container.

fn main() {
    loop {
        std::thread::park();
    }
}
