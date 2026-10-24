#!/usr/bin/env python3
"""The benchmark's CPU lanes: each lane owns whole physical cores (both SMT siblings) on one NUMA node, so lanes
measured at the same time share no core. A lane gives one core to easi.run's browser and one to its display
(the KasmVNC baseline gets both), and two more to the client that measures.

    python3 bench/runner/lanes.py N    # prints CONTROLLER_BENCH_LANES for N lanes

Start the bench stack with that value in BENCH_LANES, then run run.py with --lanes N; run.py checks the controller
was started with the same lanes.
"""

import dataclasses
import pathlib
import sys

CPU = pathlib.Path("/sys/devices/system/cpu")
NODE = pathlib.Path("/sys/devices/system/node")
# Cores left to the host on every node: interrupts, the admin, the fixture host, and unrelated work land there.
RESERVED = 2
CORES_PER_LANE = 4


@dataclasses.dataclass(frozen=True)
class Lane:
    index: int
    browser: str
    display: str
    client: str
    mems: str

    @property
    def baseline(self):
        return f"{self.browser},{self.display}"


def cpu_list(text):
    cpus = []
    for part in text.strip().split(","):
        low, _, high = part.partition("-")
        cpus += range(int(low), int(high or low) + 1)
    return cpus


def cores_by_node():
    """Each NUMA node's physical cores, each as its sibling list, in CPU order."""
    nodes = {}
    for node in sorted(NODE.glob("node[0-9]*"), key=lambda p: int(p.name[4:])):
        seen, cores = set(), []
        for cpu in cpu_list((node / "cpulist").read_text()):
            siblings = (CPU / f"cpu{cpu}" / "topology" / "thread_siblings_list").read_text().strip()
            if siblings not in seen:
                seen.add(siblings)
                cores.append(siblings)
        nodes[int(node.name[4:])] = cores
    return nodes


def layout(count):
    """`count` lanes, alternating NUMA nodes; fails when the host has too few cores."""
    free = {node: cores[RESERVED:] for node, cores in cores_by_node().items()}
    lanes = []
    order = sorted(free)
    while len(lanes) < count:
        node = order[len(lanes) % len(order)]
        if len(free[node]) < CORES_PER_LANE:
            if all(len(c) < CORES_PER_LANE for c in free.values()):
                raise SystemExit(f"this host has room for {len(lanes)} lanes, not {count}")
            order.remove(node)
            continue
        browser, display, *client = free[node][:CORES_PER_LANE]
        free[node] = free[node][CORES_PER_LANE:]
        lanes.append(Lane(len(lanes), browser, display, ",".join(client), str(node)))
    return lanes


def controller_env(lanes):
    return " ".join(f"{lane.browser}/{lane.display}/{lane.mems}" for lane in lanes)


if __name__ == "__main__":
    print(controller_env(layout(int(sys.argv[1]))))
