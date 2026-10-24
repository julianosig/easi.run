//! Keeps a process on as many neighboring physical cores as its CPU quota can use.
//!
//! A frame's work crosses many threads: Chromium's renderer, raster, and compositor threads in the browser, and a
//! dozen encoder stage threads in the display. On a many-core, multi-socket host the scheduler otherwise puts them on
//! different cores and sockets for every frame, and the data follows them through the caches. Measured on a 96-CPU
//! two-socket host, an AV1 frame cost 13 ms of CPU instead of 6, and Chromium rendered a 1080p video at 26 frames a
//! second instead of 45. Containers keep their CPU quota; this only chooses where that CPU is spent, and gives a quota
//! above one CPU the cores to spend it on.

use rustix::thread::{CpuSet, sched_getaffinity, sched_getcpu, sched_setaffinity};

/// Confines the calling thread, and every thread and process it starts later, to the physical core of the CPU it runs
/// on and, for a quota above one CPU, the next cores of the same socket: the quota rounded up, within the CPUs it is
/// allowed. Returns that CPU and the chosen cores' sibling lists, or `None` when the host does not report its core
/// topology or the socket has fewer allowed cores than the quota (placement then stays with the kernel).
pub fn stay_on_cores() -> Option<(usize, String)> {
    let allowed = sched_getaffinity(None).ok()?;
    let cpu = sched_getcpu();
    let topology = |c: usize, what: &str| {
        std::fs::read_to_string(format!("/sys/devices/system/cpu/cpu{c}/topology/{what}"))
            .ok()
            .map(|t| t.trim().to_owned())
    };
    let socket = topology(cpu, "physical_package_id")?;
    let want = std::fs::read_to_string("/sys/fs/cgroup/cpu.max").ok().and_then(|t| quota_cores(&t)).unwrap_or(1);
    let (mut chosen, mut lists) = (CpuSet::new(), Vec::new());
    // The starting CPU's core first, then the following CPUs' cores in number order, wrapping around.
    for next in (0..CpuSet::MAX_CPU).map(|i| (cpu + i) % CpuSet::MAX_CPU) {
        if lists.len() == want {
            break;
        }
        if !allowed.is_set(next) || chosen.is_set(next) || topology(next, "physical_package_id") != Some(socket.clone())
        {
            continue;
        }
        let list = topology(next, "thread_siblings_list")?;
        let core = siblings(&list, &allowed)?;
        for c in (0..CpuSet::MAX_CPU).filter(|&c| core.is_set(c)) {
            chosen.set(c);
        }
        lists.push(list);
    }
    // A socket without enough allowed cores for the quota keeps the kernel's placement rather than starve it.
    let enough = lists.len() == want;
    (enough && chosen.is_set(cpu) && sched_setaffinity(None, &chosen).is_ok()).then(|| (cpu, lists.join(" ")))
}

/// The physical cores a cgroup v2 `cpu.max` quota (`"150000 100000"`) can use: the quota rounded up; `None` without a
/// quota (`"max ..."`) or for text it does not read.
fn quota_cores(cpu_max: &str) -> Option<usize> {
    let mut fields = cpu_max.split_whitespace();
    let quota: u64 = fields.next()?.parse().ok()?;
    let period: u64 = fields.next()?.parse().ok()?;
    (period > 0).then(|| quota.div_ceil(period).max(1) as usize)
}

/// The CPUs of a sibling list (`"3,51"` or `"0-1"`) that are also allowed.
fn siblings(list: &str, allowed: &CpuSet) -> Option<CpuSet> {
    let mut core = CpuSet::new();
    for part in list.split(',') {
        let (from, to) = part.split_once('-').unwrap_or((part, part));
        let (from, to) = (from.parse::<usize>().ok()?, to.parse::<usize>().ok()?);
        for cpu in from..=to.min(CpuSet::MAX_CPU - 1) {
            if allowed.is_set(cpu) {
                core.set(cpu);
            }
        }
    }
    Some(core)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sibling_lists_parse_within_the_allowed_cpus() {
        let mut allowed = CpuSet::new();
        for cpu in [3, 4, 51] {
            allowed.set(cpu);
        }
        let core = siblings("3,51", &allowed).unwrap();
        assert!(core.is_set(3) && core.is_set(51) && !core.is_set(4));
        let core = siblings("2-4", &allowed).unwrap();
        assert!(!core.is_set(2) && core.is_set(3) && core.is_set(4));
        assert!(siblings("x", &allowed).is_none());
    }

    #[test]
    fn a_quota_rounds_up_to_whole_cores() {
        assert_eq!(quota_cores("100000 100000\n"), Some(1));
        assert_eq!(quota_cores("150000 100000"), Some(2));
        assert_eq!(quota_cores("50000 100000"), Some(1));
        assert_eq!(quota_cores("max 100000"), None);
        assert_eq!(quota_cores(""), None);
    }
}
