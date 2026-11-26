#!/usr/bin/env python3
"""Measures the agent's page read (docs/benchmark.md, Agent page reads): what `get_browser_state` in its semantic
format takes on a dense page, with the driver the agent image carries (the Cua Driver built with the patches in
scripts/patches, scripts/patches/cua-driver-occlusion.patch among them) and with that same version's release binary,
unchanged.

Needs the bench profile up with the agent and fixtures images built from this checkout, and the admin password in
ADMIN_PASSWORD. Writes `runs.jsonl`, `summary.json` and `summary.md` to the output directory.

    ADMIN_PASSWORD=... python3 bench/runner/agent_read.py --out bench/results/<date>-agent-read   # a new directory

For each page size, one session with Developer tools opens the feed fixture with that many posts. Its agent's
driver reads the page through the admin, as an agent does (`POST /api/sessions/{id}/agent`). Then each of the two
drivers runs beside the session's agent, in a container with the agent's image, user, groups, mounts, environment,
browser namespaces and CPU, memory and process limits, in turns: one driver, then the other, for `--rounds` rounds of
`--reads` reads each, so whatever else the host does falls on both alike. A read counts only as a semantic snapshot
(a refusal or an error is counted apart, and fails the run). In each turn the driver also reads the whole page, its
first piece and every continuation after it; the two drivers' whole pages, once their own identifiers (a session's,
a tab's, the read's number in a ref) are taken out, must be the same.
"""

import argparse
import hashlib
import json
import os
import pathlib
import platform
import re
import selectors
import shutil
import statistics
import subprocess
import sys
import tarfile
import tempfile
import time
import urllib.request
import uuid

import run  # the benchmark runner's admin client and the bench stack's names

RUNNER = pathlib.Path(__file__).resolve().parent
ROOT = RUNNER.parents[1]
# The release whose unchanged binary is the comparison: the version the agent image's driver is built from, and the
# SHA-256 its release publishes for that archive (the asset's digest, and its SHA256SUMS).
RELEASE_SHA256 = {
    "0.56.9": "b5605436262e7ceaaea45d138f6794d952a01d2c91c9ab94ba083071610c56d5",
    "0.62.1": "2900f9860f62e9d95994243f68695fb08d930800d1e7045ca5d98c16023631c0",
}
RELEASE_URL = ("https://github.com/trycua/cua/releases/download/cua-driver-rs-v{v}/"
               "cua-driver-rs-{v}-linux-x86_64-binary.tar.gz")
SIZES = (50, 100, 200, 400)
# Where the release's archive is kept between runs (outside the checkout), checked again each time it is used.
CACHE = pathlib.Path(os.environ.get("XDG_CACHE_HOME", pathlib.Path.home() / ".cache")) / "easi-run-bench"
# What the controller's agent-mcp sets for the driver in a session with Developer tools (agent/src/bin/agent-mcp.rs);
# its runtime directory is the container's /tmp here, a tmpfs as the agent's is, where agent-mcp makes a private
# /tmp/runtime in it.
DRIVER_ENV = {
    "CUA_DRIVER_PERMISSION_MODE": "unrestricted",
    "CUA_DRIVER_DANGEROUSLY_BYPASS_APPROVALS": "1",
    "CUA_DRIVER_CDP_PORT": "9222",
    "XDG_RUNTIME_DIR": "/tmp",
    "CUA_DRIVER_RS_TELEMETRY_ENABLED": "false",
    "DO_NOT_TRACK": "1",
    "CUA_DRIVER_RS_UPDATE_CHECK": "false",
}
# The longest a driver may take to answer one request before the run fails: past the slowest read measured.
ANSWER_WITHIN = 300
# A driver's own identifiers, which differ between two drivers reading one page: the read's tab and target, its
# number (`snapshot.id`, `p2`), which a ref starts with (`p2:14`, in a `ref` field) and its summary names ("semantic
# snapshot p2 of ..."), and its continuation, of which only whether there is one is the page's.
REF = re.compile(r"^p[0-9]+:")
SUMMARY = re.compile(r"^semantic snapshot p[0-9]+ ")


def version():
    found = re.search(r"^ARG CUA_DRIVER_VERSION=(\S+)$", (ROOT / "Dockerfile").read_text(), re.MULTILINE)
    return found.group(1)


def sha256(path):
    return hashlib.sha256(pathlib.Path(path).read_bytes()).hexdigest()


def release_binary(v, into):
    """The release's `cua-driver`, from its archive (fetched once, checked against its published SHA-256 each time),
    extracted into `into`."""
    if v not in RELEASE_SHA256:
        raise SystemExit(f"no SHA-256 for release {v}: add its published one to RELEASE_SHA256")
    archive = CACHE / f"cua-driver-rs-{v}-linux-x86_64-binary.tar.gz"
    if not archive.exists():
        CACHE.mkdir(parents=True, exist_ok=True)
        urllib.request.urlretrieve(RELEASE_URL.format(v=v), archive)
    if sha256(archive) != RELEASE_SHA256[v]:
        archive.unlink()
        raise SystemExit(f"the release archive of {v} does not match its SHA-256")
    with tarfile.open(archive) as tar:
        tar.extract(tar.getmember("cua-driver"), into, filter="data")
    path = pathlib.Path(into) / "cua-driver"
    path.chmod(0o755)
    return path


def copied(image, inside, path):
    """The file `inside` the image `image`, copied to `path` (made readable: the agent's driver is execute-only)."""
    container = subprocess.run(["docker", "create", image, "none"], check=True, capture_output=True,
                               text=True).stdout.strip()
    try:
        subprocess.run(["docker", "cp", "-q", f"{container}:{inside}", str(path)], check=True)
        path.chmod(0o755)
        return path
    finally:
        subprocess.run(["docker", "rm", "-f", container], check=True, capture_output=True)


def pinned_driver(into):
    """The driver the root Dockerfile pins (CUA_DRIVER), from its published image, which this run's agent images must
    carry."""
    image = re.search(r"^ARG CUA_DRIVER=(\S+)$", (ROOT / "Dockerfile").read_text(), re.MULTILINE).group(1)
    if image == "driver":
        raise SystemExit("the Dockerfile pins no published driver (CUA_DRIVER=driver)")
    subprocess.run(["docker", "pull", "-q", image], check=True, capture_output=True)
    return image, copied(image, "/cua/cua-driver", pathlib.Path(into) / "pinned-cua-driver")


def inspect(container):
    inspected = subprocess.run(["docker", "inspect", container], check=True, capture_output=True, text=True)
    return json.loads(inspected.stdout)[0]


def beside(agent, name, binary):
    """`docker run` for a driver in a container named `name` with `agent`'s settings, `binary` (a host path) mounted in
    as its program, or the image's own driver when None."""
    host = agent["HostConfig"]
    command = ["docker", "run", "-i", "--rm", "--name", name, "--read-only",
               "--tmpfs", "/tmp:rw,noexec,nosuid,nodev,size=64m", "--user", agent["Config"]["User"],
               "--network", host["NetworkMode"], "--pid", host["PidMode"],
               "--cap-drop", "ALL", "--security-opt", "no-new-privileges:true"]
    if host.get("CpuQuota"):
        command += ["--cpu-period", str(host["CpuPeriod"]), "--cpu-quota", str(host["CpuQuota"])]
    elif host.get("NanoCpus"):
        command += ["--cpus", str(host["NanoCpus"] / 1e9)]
    if host.get("Memory"):
        command += ["--memory", str(host["Memory"]), "--memory-swap", str(host.get("MemorySwap") or host["Memory"])]
    if host.get("PidsLimit"):
        command += ["--pids-limit", str(host["PidsLimit"])]
    for limit in host.get("Ulimits") or []:
        command += ["--ulimit", f"{limit['Name']}={limit['Soft']}:{limit['Hard']}"]
    for group in host.get("GroupAdd") or []:
        command += ["--group-add", group]
    for mount in agent["Mounts"]:
        source = mount["Name"] if mount["Type"] == "volume" else mount["Source"]
        command += ["--mount", f"type={mount['Type']},src={source},dst={mount['Destination']},readonly"]
    env = [e for e in agent["Config"]["Env"] if not e.startswith("AGENT_")]
    for e in env + [f"{k}={v}" for k, v in DRIVER_ENV.items()]:
        command += ["--env", e]
    program = "/usr/local/bin/cua-driver"
    if binary is not None:
        command += ["--mount", f"type=bind,src={binary},dst=/opt/release/cua-driver,readonly"]
        program = "/opt/release/cua-driver"
    return command + ["--entrypoint", program, agent["Config"]["Image"], "mcp", "--direct"]


class Driver:
    """A driver's MCP server on its standard streams, in a container of its own that goes with it."""

    def __init__(self, agent, binary):
        self.name = f"agent-read-{uuid.uuid4().hex[:12]}"
        self.process = subprocess.Popen(beside(agent, self.name, binary), stdin=subprocess.PIPE,
                                        stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        self.readable = selectors.DefaultSelector()
        self.readable.register(self.process.stdout, selectors.EVENT_READ)
        self.pending = b""
        self.id = 0

    def start(self):
        self.ask("initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                                "clientInfo": {"name": "agent_read.py", "version": "1"}})
        self.send({"jsonrpc": "2.0", "method": "notifications/initialized"})

    def send(self, message):
        self.process.stdin.write((json.dumps(message) + "\n").encode())
        self.process.stdin.flush()

    def line(self, deadline):
        """The driver's next line, read as it comes and never past `deadline`."""
        while b"\n" not in self.pending:
            left = deadline - time.monotonic()
            if left <= 0 or not self.readable.select(timeout=left):
                raise RuntimeError(f"no answer within {ANSWER_WITHIN} s")
            chunk = os.read(self.process.stdout.fileno(), 1 << 20)
            if not chunk:
                raise RuntimeError("the driver closed its output")
            self.pending += chunk
        if time.monotonic() > deadline:
            raise RuntimeError(f"no answer within {ANSWER_WITHIN} s")
        line, self.pending = self.pending.split(b"\n", 1)
        return line

    def ask(self, method, params):
        self.id += 1
        self.send({"jsonrpc": "2.0", "id": self.id, "method": method, "params": params})
        deadline = time.monotonic() + ANSWER_WITHIN
        while True:
            message = json.loads(self.line(deadline))
            if message.get("id") == self.id:
                if "error" in message:
                    raise RuntimeError(f"{method}: {message['error']}")
                return message["result"]

    def tool(self, name, arguments):
        return self.ask("tools/call", {"name": name, "arguments": arguments})

    def close(self):
        subprocess.run(["docker", "rm", "-f", self.name], check=False, capture_output=True)
        self.process.kill()
        self.process.wait()


def bound(call, label):
    """The page's tab, bound for reads through `call` (a tool's name and arguments to its result): the read's
    arguments."""
    windows = call("list_windows", {})["structuredContent"]["windows"]
    window = next(w for w in windows if w.get("pid"))
    native = {"pid": window["pid"], "window_id": window["window_id"], "session": label}
    prepared = call("browser_prepare", native | {"strategy": {"kind": "existing_profile"}})
    if prepared["structuredContent"].get("status") != "ok":
        raise RuntimeError(f"browser_prepare: {prepared['structuredContent']}")
    state = call("get_browser_state", native)["structuredContent"]
    tab = next(t for t in state["tabs"] if t.get("active"))
    return {"session": label, "target_id": state["target_id"], "tab_id": tab["tab_id"],
            "snapshot_format": "semantic_v2"}


def snapshot(result):
    """The read's semantic snapshot, or None when it is not one (an error, a refusal, another format)."""
    content = result.get("structuredContent")
    if result.get("isError") or not isinstance(content, dict) or content.get("status") != "ok":
        return None
    shot = content.get("snapshot")
    return content if isinstance(shot, dict) and shot.get("format") == "semantic_v2" else None


def refs(value):
    """`value` with every ref's read number taken out (`p2:14` as `-:14`), in the fields that hold refs alone."""
    if isinstance(value, dict):
        return {k: REF.sub("-:", v) if (k == "ref" or k.endswith("_ref")) and isinstance(v, str) else refs(v)
                for k, v in value.items()}
    if isinstance(value, list):
        return [refs(v) for v in value]
    return value


def said(result):
    """What a read says of the page, its text included, without the driver's own identifiers."""
    content = refs(json.loads(json.dumps(result["structuredContent"])))
    content["tab_id"] = content["target_id"] = "-"
    content["snapshot"]["id"] = "-"
    content["snapshot"]["continuation"] = content["snapshot"].get("continuation") is not None
    text = [SUMMARY.sub("semantic snapshot - ", c.get("text", "")) for c in result.get("content", [])]
    return hashlib.sha256(json.dumps([content, text], sort_keys=True).encode()).hexdigest()[:16]


def whole(call, read):
    """What the driver says of the whole page, its first piece and every continuation after it, followed at once,
    with the time all of it took; None when a piece is no snapshot or the page does not end whole."""
    pieces = []
    started = time.monotonic()
    result = call("get_browser_state", read)
    while True:
        content = snapshot(result)
        if content is None:
            return None
        pieces.append(said(result))
        continuation = content["snapshot"].get("continuation")
        if not continuation:
            if not content["snapshot"].get("complete"):
                return None
            ms = round((time.monotonic() - started) * 1000, 1)
            return hashlib.sha256("".join(pieces).encode()).hexdigest()[:16], len(pieces), ms
        result = call("get_browser_state", read | {"continuation": continuation})


def timed(call, read, reads):
    runs = []
    for _ in range(reads):
        started = time.monotonic()
        result = call("get_browser_state", read)
        ms = (time.monotonic() - started) * 1000
        content = snapshot(result)
        # A first piece, whole: one with a continuation, or the whole page in one.
        if content and not (content["snapshot"].get("continuation") or content["snapshot"].get("complete")):
            content = None
        runs.append({"ms": round(ms, 1), "bytes": len(json.dumps(result)), "ok": content is not None,
                     "nodes": content["snapshot"].get("total_nodes") if content else None,
                     "said": said(result) if content else None})
    return runs


def through_admin(admin, session):
    def call(tool, arguments):
        return admin.call("POST", f"/api/sessions/{session}/agent", {"tool": tool, "arguments": arguments})
    return call


def wait_loaded(call, posts):
    """Until the page's tree holds its posts: a read's element count stops growing."""
    deadline = time.monotonic() + 120
    before = None
    while time.monotonic() < deadline:
        try:
            read = bound(call, "load")
        except (RuntimeError, StopIteration, KeyError):
            time.sleep(1)
            continue
        content = snapshot(call("get_browser_state", read))
        total = content["snapshot"].get("total_nodes") if content else None
        if total and total > posts and total == before:
            return
        before = total
        time.sleep(1)
    raise RuntimeError(f"the feed of {posts} posts never settled")


def measure(admin, posts, rounds, reads, release, checked):
    created = admin.call("POST", "/api/sessions", {"url": f"https://fixture.bench/feed.html?n={posts}", "devtools": True})
    session = created["id"]
    try:
        admin_call = through_admin(admin, session)
        wait_loaded(admin_call, posts)
        agent = inspect(f"{run.INSTANCE}-{session}-agent")
        checked(agent["Image"])  # the session's own image, by its id: what its drivers run, before any is timed
        runs = [{"posts": posts, "driver": "patched", "path": "admin", **r}
                for r in timed(admin_call, bound(admin_call, "bench"), reads)]
        for round_ in range(rounds):
            for name, binary in (("release", release), ("patched", None)) if round_ % 2 == 0 else (
                    ("patched", None), ("release", release)):
                driver = Driver(agent, binary)
                try:
                    driver.start()
                    read = bound(driver.tool, "bench")
                    page = whole(driver.tool, read)  # also warms what the driver keeps
                    runs.append({"posts": posts, "driver": name, "path": "whole", "round": round_,
                                 "ok": page is not None, "said": page and page[0], "pieces": page and page[1],
                                 "whole_ms": page and page[2]})
                    runs += [{"posts": posts, "driver": name, "path": "beside", "round": round_, **r}
                             for r in timed(driver.tool, read, reads)]
                finally:
                    driver.close()
        return runs
    finally:
        admin.call("DELETE", f"/api/sessions/{session}")


def summarize(runs):
    summary = []
    for posts in sorted({r["posts"] for r in runs}):
        row = {"posts": posts}
        page = [r for r in runs if r["posts"] == posts]
        for key, match in (("release", ("release", "beside")), ("patched", ("patched", "beside")),
                           ("admin", ("patched", "admin"))):
            ms = sorted(r["ms"] for r in page if (r["driver"], r["path"]) == match and r["ok"])
            if ms:
                row[key] = {"median_ms": statistics.median(ms), "p95_ms": ms[min(len(ms) - 1, int(len(ms) * 0.95))],
                            "runs": len(ms)}
        for key, name in (("release_whole", "release"), ("patched_whole", "patched")):
            ms = sorted(r["whole_ms"] for r in page if r["driver"] == name and r["path"] == "whole" and r["ok"])
            if ms:
                row[key] = {"median_ms": statistics.median(ms), "runs": len(ms)}
        row["failed"] = sum(not r["ok"] for r in page)
        row["nodes"] = max((r.get("nodes") or 0) for r in page)
        row["pieces"] = max((r.get("pieces") or 0) for r in page)
        said_by = {name: {r["said"] for r in page if r["driver"] == name and r["path"] == "whole"}
                   for name in ("release", "patched")}
        first_by = {name: {r["said"] for r in page if r["driver"] == name and r["path"] == "beside"}
                    for name in ("release", "patched")}
        row["same_page"] = (row["failed"] == 0 and len(said_by["release"]) == 1 and said_by["release"] == said_by["patched"]
                            and len(first_by["release"]) == 1 and first_by["release"] == first_by["patched"])
        if "release" in row and "patched" in row:
            row["speedup"] = round(row["release"]["median_ms"] / row["patched"]["median_ms"], 1)
        summary.append(row)
    return summary


def seconds(m):
    return f"{m['median_ms'] / 1000:.2f} s" if m else "—"


def markdown(summary, machine):
    lines = ["# Agent page reads", "", machine, "",
             ("| posts | elements | release, first piece: median (p95) | patched, first piece: median (p95) "
              "| through the admin | speedup | whole page: release, patched | same whole page | failed reads |"),
             "|---|---|---|---|---|---|---|---|---|"]
    for row in summary:
        def cell(key, row=row):
            m = row.get(key)
            return f"{m['median_ms'] / 1000:.2f} s ({m['p95_ms'] / 1000:.2f} s)" if m else "—"
        lines.append(f"| {row['posts']} | {row['nodes']} | {cell('release')} | {cell('patched')} | {cell('admin')} | "
                     f"{row.get('speedup', '—')}x | {seconds(row.get('release_whole'))}, "
                     f"{seconds(row.get('patched_whole'))} | {'yes' if row['same_page'] else 'no'} "
                     f"({row['pieces']} pieces) | {row['failed']} |")
    return "\n".join(lines) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--out", required=True, type=pathlib.Path)
    parser.add_argument("--sizes", default=",".join(map(str, SIZES)), help="comma-separated post counts")
    parser.add_argument("--rounds", type=int, default=4, help="turns of the two drivers on each page (even)")
    parser.add_argument("--reads", type=int, default=3, help="timed reads in each turn")
    args = parser.parse_args()
    password = os.environ.get("ADMIN_PASSWORD") or sys.exit("ADMIN_PASSWORD is not set")
    if args.out.exists() or args.out.is_symlink():
        sys.exit(f"{args.out} exists: name a new directory for this run's results")
    args.out.parent.mkdir(parents=True, exist_ok=True)
    v = version()
    admin = run.Admin(password)
    with tempfile.TemporaryDirectory() as scratch:
        release = release_binary(v, scratch)
        pinned, patched = pinned_driver(scratch)
        drivers = {}
        for name, path in (("release", release), ("patched", patched)):
            # Asked in the agent's image, which has the libraries the driver loads.
            said_version = subprocess.run(["docker", "run", "--rm", "--network", "none", "--mount",
                                           f"type=bind,src={path},dst=/opt/driver,readonly", "--entrypoint",
                                           "/opt/driver", run.PLAYWRIGHT, "--version"],
                                          check=True, capture_output=True, text=True)
            drivers[name] = {"version": said_version.stdout.strip(), "sha256": sha256(path)}
        if drivers["patched"]["version"] != f"cua-driver {v}" or drivers["release"]["version"] != f"cua-driver {v}":
            raise SystemExit(f"the drivers are not this checkout's version {v}: {drivers}")
        if drivers["patched"]["sha256"] == drivers["release"]["sha256"]:
            raise SystemExit("the pinned driver is the release's own binary, not the patched one")
        images = set()

        def checked(image):
            """Every session's agent image carries the pinned driver: what runs as "patched" is it."""
            if image not in images:
                carried = copied(image, "/usr/local/bin/cua-driver", pathlib.Path(scratch) / "carried")
                if sha256(carried) != drivers["patched"]["sha256"]:
                    raise SystemExit(f"the sessions' agent image {image} does not carry the pinned driver {pinned}")
                images.add(image)
            if len(images) != 1:
                raise SystemExit(f"the sessions run on several agent images: {sorted(images)}")

        runs = []
        for posts in map(int, args.sizes.split(",")):
            runs += measure(admin, posts, args.rounds, args.reads, release, checked)
            print(f"{posts} posts measured", flush=True)
    image_id = images.pop()
    machine = (f"Measured {time.strftime('%Y-%m-%d')} on {platform.machine()}, {os.cpu_count()} CPUs, Linux "
               f"{platform.release()}: Cua Driver {v}, its release's binary (SHA-256 {drivers['release']['sha256'][:12]}) "
               f"and the pinned one the sessions' agent image ({image_id[:19]}) carries (SHA-256 "
               f"{drivers['patched']['sha256'][:12]}), each under the agent's limits; {args.rounds} turns of "
               f"{args.reads} timed reads for each driver and page.")
    summary = summarize(runs)
    if any(row["failed"] or not row["same_page"] for row in summary):
        print(markdown(summary, machine))
        sys.exit("a read failed, or the two drivers did not say the same of a page: nothing written")
    # Written only now, whole, in a directory beside the output that then becomes it: a run that failed, or a write
    # that did, leaves nothing at the output.
    writing = pathlib.Path(tempfile.mkdtemp(prefix=f".{args.out.name}.", dir=args.out.parent))
    try:
        (writing / "runs.jsonl").write_text("".join(json.dumps(r) + "\n" for r in runs))
        (writing / "summary.json").write_text(json.dumps({"machine": machine, "version": v, "pinned_driver": pinned,
                                                          "agent_image_id": image_id, "drivers": drivers,
                                                          "pages": summary}, indent=2) + "\n")
        (writing / "summary.md").write_text(markdown(summary, machine))
        umask = os.umask(0)
        os.umask(umask)
        writing.chmod(0o777 & ~umask)  # as a directory made at the output would be (mkdtemp makes it private)
        writing.rename(args.out)
    except BaseException:
        shutil.rmtree(writing, ignore_errors=True)
        raise
    print(markdown(summary, machine))


if __name__ == "__main__":
    main()
