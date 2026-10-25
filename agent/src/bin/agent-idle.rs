//! The agent container's main process: it waits, so the controller can execute the driver in the container.

fn main() {
    // An execute-only program is as dumpable as the host's fs.suid_dumpable says; this makes it non-dumpable on every
    // host: the kernel makes no core dump of it, and no other process of its uid may read its memory.
    if let Err(why) = rustix::process::set_dumpable_behavior(rustix::process::DumpableBehavior::NotDumpable) {
        eprintln!("agent-idle: cannot make itself non-dumpable: {why}");
        std::process::exit(1);
    }
    loop {
        std::thread::park();
    }
}
