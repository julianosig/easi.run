//! The browser's seccomp profile (browser/seccomp.json) lets the browser open sockets only of the families it needs:
//! evaluated from the profile itself, so a rule widened or added anywhere in the file fails here. Needs no stack.

use serde_json::Value;

const AF_UNIX: u64 = 1;
const AF_INET6: u64 = 10;
const AF_NETLINK: u64 = 16;
const NETLINK_ROUTE: u64 = 0;
const NETLINK_KOBJECT_UEVENT: u64 = 15;

fn profile() -> Value {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../browser/seccomp.json");
    let text = std::fs::read_to_string(path).expect("browser/seccomp.json");
    serde_json::from_str(&text).expect("the profile is JSON")
}

/// The rules that name `syscall`.
fn rules<'a>(profile: &'a Value, syscall: &str) -> Vec<&'a Value> {
    let all = profile["syscalls"].as_array().expect("a syscall list");
    all.iter().filter(|rule| rule["names"].as_array().expect("names").iter().any(|n| n == syscall)).collect()
}

/// Whether one of `rules` allows a call with `args`. A rule that is not a plain allow, or compares in a way this
/// does not know, fails the test rather than being guessed at.
fn allows(rules: &[&Value], args: &[u64]) -> bool {
    rules.iter().any(|rule| {
        assert_eq!(rule["action"], "SCMP_ACT_ALLOW", "an unexpected action: {rule}");
        let conditions = rule.get("args").and_then(Value::as_array).cloned().unwrap_or_default();
        conditions.iter().all(|c| {
            let index = c["index"].as_u64().expect("an argument index") as usize;
            let value = c["value"].as_u64().expect("a comparison value");
            let arg = *args.get(index).unwrap_or_else(|| panic!("no argument {index} in {args:?}: {rule}"));
            match c["op"].as_str().expect("a comparison") {
                "SCMP_CMP_EQ" => arg == value,
                "SCMP_CMP_LT" => arg < value,
                op => panic!("an unknown comparison {op}: {rule}"),
            }
        })
    })
}

// Families and protocols tried: every family Linux defines and well past it, the low protocols, and a large one.
const FAMILIES: std::ops::Range<u64> = 0..64;
const PROTOCOLS: [u64; 6] = [0, 1, 6, 15, 17, 255];

#[test]
fn the_browser_opens_sockets_only_of_the_families_it_needs() {
    let profile = profile();
    assert_eq!(profile["defaultAction"], "SCMP_ACT_ERRNO", "what no rule allows is refused");
    let socket = rules(&profile, "socket");
    for family in FAMILIES {
        for protocol in PROTOCOLS {
            // AF_UNSPEC, AF_UNIX, and AF_INET (below 3); AF_INET6; AF_NETLINK only as route or uevent.
            let expected = family < 3
                || family == AF_INET6
                || (family == AF_NETLINK && [NETLINK_ROUTE, NETLINK_KOBJECT_UEVENT].contains(&protocol));
            let allowed = allows(&socket, &[family, 1, protocol]);
            assert_eq!(allowed, expected, "socket(family {family}, protocol {protocol})");
        }
    }
}

#[test]
fn socket_pairs_are_unix_only() {
    let profile = profile();
    let socketpair = rules(&profile, "socketpair");
    for family in FAMILIES {
        for protocol in PROTOCOLS {
            let allowed = allows(&socketpair, &[family, 1, protocol, 0]);
            assert_eq!(allowed, family == AF_UNIX, "socketpair(family {family}, protocol {protocol})");
        }
    }
}

/// The 32-bit x86 ABI multiplexes every socket call through `socketcall`, whose arguments sit in memory a filter
/// cannot read, and io_uring's `IORING_OP_SOCKET` opens a socket of any family without the `socket` call: neither those
/// calls nor that ABI is allowed.
#[test]
fn no_call_or_abi_reaches_sockets_around_the_rules() {
    let profile = profile();
    for call in ["socketcall", "io_uring_setup", "io_uring_enter", "io_uring_register"] {
        assert!(rules(&profile, call).is_empty(), "{call} is allowed");
    }
    let mut arches: Vec<&str> = Vec::new();
    for arch in profile.get("architectures").and_then(Value::as_array).into_iter().flatten() {
        arches.push(arch.as_str().expect("an architecture"));
    }
    for entry in profile.get("archMap").and_then(Value::as_array).into_iter().flatten() {
        arches.push(entry["architecture"].as_str().expect("an architecture"));
        for sub in entry["subArchitectures"].as_array().into_iter().flatten() {
            arches.push(sub.as_str().expect("a sub-architecture"));
        }
    }
    assert!(!arches.is_empty(), "the profile names its architectures");
    assert!(!arches.contains(&"SCMP_ARCH_X86"), "the 32-bit x86 ABI is allowed: {arches:?}");
}
