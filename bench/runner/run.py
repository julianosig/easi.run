#!/usr/bin/env python3
"""Runs the benchmark of docs/design.md §8: every system, every fixture, headed and headless repetitions.

Needs the bench profile up (`docker compose -p easi-run-bench -f compose.yaml -f compose.bench.yaml up -d --build`),
the tools image (`docker build -t easi-run/tools:dev bench/tools`), the runner's node modules (`npm ci` in this
directory, inside the Playwright image), and the admin password in ADMIN_PASSWORD. Writes `runs.jsonl`,
`summary.json`, and `summary.md` to the output directory; quality frames stay beside them for the SSIM step.

    ADMIN_PASSWORD=... python3 bench/runner/run.py --out bench/results/<date>    # --help lists the options

With --lanes N, N runs measure at once, each in its own CPU lane (lanes.py), the systems of every block shuffled
across lanes so contention falls on all of them alike; SSIM is computed after the last measurement. The bench stack
must be started with BENCH_LANES=$(python3 bench/runner/lanes.py N). --lanes 0 measures one run at a time, unplaced.
"""

import argparse
import concurrent.futures
import json
import os
import pathlib
import platform
import queue
import random
import re
import ssl
import statistics
import subprocess
import time
import urllib.error
import urllib.request

import lanes  # the runner's own modules: Python puts this script's directory first on the path
import reference

RUNNER = pathlib.Path(__file__).resolve().parent
ROOT = RUNNER.parents[1]

PLAYWRIGHT = "mcr.microsoft.com/playwright:v1.61.1-noble@sha256:5b8f294aff9041b7191c34a4bab3ac270157a28774d4b0660e9743297b697e48"
TOOLS = "easi-run/tools:dev"
BASELINE_IMAGE = "easi-run-bench/baseline:dev"
# The baseline image this run measures, pinned by id at its start (a rebuilt tag cannot change it mid-run); the
# controller pins easi.run's images itself when it starts.
PINNED = {}
BASELINE = "easirun-bench-baseline"
# The bench stack's (compose.bench.yaml): its Compose project's names and its controller's instance.
CONTROLLER = "easi-run-bench-controller-1"
BASELINE_NETWORK = "easi-run-bench_baseline"
BASELINE_PROXY = "http://easi-run-bench-baseline-proxy-1:3128"
BENCH_VOLUME = "easi-run-bench_bench"
INSTANCE = "easirun-bench"
ORIGIN = "https://localhost:9444"
FIXTURES = ["article", "scroll", "transform", "animation", "video", "typing", "text-scroll", "quality"]
# Auto encodes motion as AV1 for a client that decodes it; `easi-run-h264` repeats the motion fixtures with H.264.
MOTION = {"scroll", "transform", "animation", "video", "text-scroll", "quality"}
WARMUP = 5
LOAD = 8


class Admin:
    """easi.run's admin API, as the person's browser uses it."""

    def __init__(self, password):
        self.tls = ssl.create_default_context()
        self.tls.check_hostname = False
        self.tls.verify_mode = ssl.CERT_NONE
        self.cookie = ""
        self.csrf = ""
        for attempt in range(30):  # the admin may still be starting
            try:
                response = self.call("POST", "/api/login", {"password": password}, raw=True)
                break
            except (urllib.error.URLError, ConnectionError):
                if attempt == 29:
                    raise
                time.sleep(1)
        self.cookie = next(c.split(";")[0] for c in response.headers.get_all("Set-Cookie") if c.startswith("__Host-display="))
        self.csrf = json.loads(response.read())["csrf"]

    def call(self, method, path, body=None, raw=False):
        data = None if body is None else json.dumps(body).encode()
        request = urllib.request.Request(ORIGIN + path, data=data, method=method)
        request.add_header("origin", ORIGIN)
        if data is not None:
            request.add_header("content-type", "application/json")
        if self.cookie:
            request.add_header("cookie", self.cookie)
            request.add_header("x-csrf-token", self.csrf)
        for attempt in range(5):  # a controller that just restarted answers 502 for a few seconds
            try:
                response = urllib.request.urlopen(request, context=self.tls, timeout=60)
                break
            except urllib.error.HTTPError as e:
                if e.code != 502 or attempt == 4:
                    raise
                time.sleep(3)
        if raw:
            return response
        text = response.read()
        return json.loads(text) if text else None

    def clear(self):
        for session in self.call("GET", "/api/sessions")["sessions"]:
            try:
                self.call("DELETE", f"/api/sessions/{session['id']}")
            except urllib.error.HTTPError as e:
                if e.code != 404:  # already gone, as when a retried delete had succeeded
                    raise

    def create(self, url, lane=None):
        """The new session, as the admin describes it (its id, and the settings it starts with among the rest)."""
        # Kiosk mode: the page alone on the screen, as on the baseline's browser (the bench profile only).
        body = {"url": url, "devtools": False, "kiosk": True} | ({} if lane is None else {"lane": lane})
        return self.call("POST", "/api/sessions", body)

    def login(self):
        """The login cookie's value and CSRF token, which every measuring client shares."""
        return self.cookie.split("=", 1)[1], self.csrf


def docker(*args, check=True):
    return subprocess.run(["docker", *args], check=check, capture_output=True, text=True).stdout.strip()


def cgroup(container):
    """The container's cgroup (v2) directory, found through its main process: where Docker puts it depends on its
    cgroup driver."""
    pid = docker("inspect", "-f", "{{.State.Pid}}", container)
    entry = pathlib.Path(f"/proc/{pid}/cgroup").read_text().strip()
    if not entry.startswith("0::/"):
        raise SystemExit(f"{container}: not a cgroup v2 path: {entry}")
    return pathlib.Path("/sys/fs/cgroup" + entry.removeprefix("0::"))


def cpu_seconds(groups):
    total = 0
    for group in groups:
        stat = (group / "cpu.stat").read_text()
        total += int(re.search(r"^usage_usec (\d+)$", stat, re.M).group(1))
    return total / 1e6


def memory_bytes(groups):
    """The screen path's memory at this moment, as `docker stats` counts it (usage minus inactive page cache)."""
    total = 0
    for group in groups:
        stat = (group / "memory.stat").read_text()
        inactive = int(re.search(r"^inactive_file (\d+)$", stat, re.M).group(1))
        total += int((group / "memory.current").read_text()) - inactive
    return total


def placed(lane, cpus):
    """`docker run` options that keep a container on `cpus` of the lane's node; none outside a lane."""
    return [] if lane is None else ["--cpuset-cpus", cpus, "--cpuset-mems", lane.mems]


def measure(system, target, session, fixture, seconds, headed, out, network, groups, lane, login):
    """Runs measure.mjs once and returns its result with the CPU the screen path used during the window."""
    out.mkdir(parents=True, exist_ok=True)
    node = ["node", "measure.mjs", system, target, session or "-", fixture, str(seconds), str(WARMUP), "/out"]
    cookie, csrf = login or ("-", "-")
    command = ["docker", "run", "--rm", "--init", "--network", network, "-u", f"{os.getuid()}:{os.getgid()}", "-e", "HOME=/tmp",
               "-e", f"HEADED={1 if headed else 0}", "-e", "LOGIN_COOKIE", "-e", "LOGIN_CSRF",
               *placed(lane, lane.client if lane else ""), "-v", f"{RUNNER}:/w:ro", "-v", f"{out}:/out", "-w", "/w",
               PLAYWRIGHT]
    command += ["xvfb-run", "-a", "-s", "-screen 0 1920x1080x24", *node] if headed else node
    # The login travels in docker's environment, not on its command line, where every local user could read it.
    env = os.environ | {"LOGIN_COOKIE": cookie, "LOGIN_CSRF": csrf}
    process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, env=env)
    marks, lines, host, memory = {}, [], [], []
    try:
        for line in process.stdout:
            lines.append(line.rstrip())
            if line.startswith("MARK "):
                marks[line.split()[1]] = (time.monotonic(), cpu_seconds(groups))
                host.append(host_state(lane))
                memory.append(memory_bytes(groups))
    except BaseException:
        # docker run forwards the signal, so the measuring container stops and is removed with it.
        process.terminate()
        process.wait(timeout=60)
        raise
    process.wait()
    if process.returncode != 0 or "start" not in marks or "end" not in marks:
        raise RuntimeError("\n".join(lines[-20:]))
    result = json.loads((out / "result.json").read_text())
    (t0, c0), (t1, c1) = marks["start"], marks["end"]
    result["cpuCores"] = (c1 - c0) / (t1 - t0)
    result["host"] = host
    # The screen path's memory at the end of the window, and the sum of each container's own peak since it started
    # (an upper bound on the path's peak: the containers may peak at different times).
    result["memoryBytes"] = memory[-1]
    result["memoryPeakBytes"] = sum(int((group / "memory.peak").read_text()) for group in groups)
    return result


def run_easi_run(admin, fixture, seconds, headed, out, lane, codec="auto", bitrate=None):
    created = admin.create(f"https://fixture.bench/{fixture}.html", None if lane is None else lane.index)
    session = created["id"]
    try:
        if codec != "auto" or bitrate is not None:
            # The session's own settings, with the codec and the bitrate asked for in place of its own.
            settings = created["settings"] | {"codec": codec} | ({} if bitrate is None else {"bitrate_kbps": bitrate})
            admin.call("PUT", f"/api/sessions/{session}/settings", settings)
        time.sleep(LOAD)
        groups = [cgroup(f"{INSTANCE}-{session}-browser"), cgroup(f"{INSTANCE}-{session}-display")]
        result = measure("easi-run", ORIGIN, session, fixture, seconds, headed, out, "host", groups, lane,
                         admin.login())
        result["displayMemoryPeakBytes"] = int((groups[1] / "memory.peak").read_text())
        result["browserImage"] = docker("inspect", "-f", "{{.Image}}", f"{INSTANCE}-{session}-browser")
        return result
    finally:
        admin.call("DELETE", f"/api/sessions/{session}")


def run_baseline(fixture, seconds, headed, out, lane):
    name = BASELINE if lane is None else f"{BASELINE}-{lane.index}"
    docker("rm", "-f", name, check=False)
    docker("run", "-d", "--name", name, "--network", BASELINE_NETWORK, *placed(lane, lane.baseline if lane else ""),
           "--read-only",
           "--cap-drop", "ALL", "--security-opt", "no-new-privileges", "--security-opt",
           f"seccomp={ROOT / 'browser/seccomp.json'}", "--init", "--shm-size", "512m", "--cpus", "2", "--memory", "2g",
           "--tmpfs", "/tmp:size=512m", "--tmpfs", "/home/browser:size=512m,uid=10001,gid=10001",
           "-v", f"{BENCH_VOLUME}:/run/bench:ro", "-e", f"BROWSER_URL=https://fixture.bench/{fixture}.html",
           "-e", f"BROWSER_PROXY={BASELINE_PROXY}", PINNED.get("baseline", BASELINE_IMAGE))
    try:
        time.sleep(LOAD + 2)
        result = measure("baseline", f"https://{name}:6901", None, fixture, seconds, headed, out, BASELINE_NETWORK,
                         [cgroup(name)], lane, None)
        result["browserImage"] = docker("inspect", "-f", "{{.Image}}", name)
        return result
    finally:
        docker("rm", "-f", name, check=False)


def ssim(out, fixture, stamps):
    """SSIM (all planes) of each captured frame against the fixture's exact reference frame."""
    for stamp in stamps:
        reference.png(reference.FRAMES[fixture](stamp), out / f"ref-{stamp}.png")
    script = 'for f in frame-*.png; do s=${f##*-}; ffmpeg -nostdin -loglevel info -i "$f" -i "ref-$s" ' \
             '-lavfi "[0:v]format=rgb24[a];[1:v]format=rgb24[b];[a][b]ssim" -f null - 2>&1 | grep -o "All:[0-9.]*"; done'
    text = docker("run", "--rm", "-u", f"{os.getuid()}:{os.getgid()}", "-v", f"{out}:/f", "-w", "/f",
                  "--entrypoint", "sh", TOOLS, "-c", script)
    return [float(v.split(":")[1]) for v in text.split()]


def percentile(values, p):
    if not values:
        return None
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, int(round(p / 100 * (len(ordered) - 1))))]


def median(values):
    values = [v for v in values if v is not None]
    return statistics.median(values) if values else None


def summarize(runs):
    summary = {}
    for (system, fixture, headed), group in sorted(_groups(runs).items()):
        ok = [r for r in group if "error" not in r]
        row = {
            "runs": len(group),
            "failed": len(group) - len(ok),
            "presentedFps": median([r["presentedFps"] for r in ok]),
            "kbitPerSecond": median([r["kbitPerSecond"] for r in ok]),
            "mediaBytesPerSecond": median([r["mediaBytesPerSecond"] for r in ok]),
            "captureToDrawP50": median([percentile(r["latencies"], 50) for r in ok]),
            "captureToDrawP95": median([percentile(r["latencies"], 95) for r in ok]),
            "inputToDrawP50": median([percentile(r["inputLatencies"], 50) for r in ok]),
            "inputToDrawP95": median([percentile(r["inputLatencies"], 95) for r in ok]),
            "cpuCores": median([r["cpuCores"] for r in ok]),
            "fpsPerCpu": median([r["presentedFps"] / r["cpuCores"] for r in ok if r["cpuCores"] > 0]),
            "ssim": median([statistics.mean(r["ssim"]) for r in ok if r.get("ssim")]),
            "displayMemoryPeakMiB": median([r["displayMemoryPeakBytes"] / 2**20 for r in ok if "displayMemoryPeakBytes" in r]),
            "memoryMiB": median([r["memoryBytes"] / 2**20 for r in ok if "memoryBytes" in r]),
            "memoryPeakMiB": median([r["memoryPeakBytes"] / 2**20 for r in ok if "memoryPeakBytes" in r]),
            "clockErrorMs": median([r["clockErrorUs"] / 1000 for r in ok if r.get("clockErrorUs") is not None]),
            "stagesMs": {stage: median([r["stages"][stage] for r in ok if r.get("stages")])
                         for stage in ("composeToArrive", "composeToDraw", "decode", "encode")},
        }
        summary.setdefault(f"{system}/{'headed' if headed else 'headless'}", {})[fixture] = row
    return summary


def _groups(runs):
    groups = {}
    for r in runs:
        groups.setdefault((r["system"], r["fixture"], r["headed"]), []).append(r)
    return groups


def fmt(v, digits=1):
    return "–" if v is None else f"{v:.{digits}f}"


def markdown(summary, machine):
    lines = ["# Benchmark (docs/design.md §8)", "", "Machine: " + "; ".join(f"{k}: {v}" for k, v in machine.items()), ""]
    for mode in ("headed", "headless"):
        lines += [f"## {mode.capitalize()} client", "",
                  "| System | Fixture | Runs (failed) | Presented fps | kbit/s | Media B/s | Capture-to-draw p50/p95 ms "
                  "| Input-to-draw p50/p95 ms | CPU cores | fps/CPU | SSIM | screen MiB (sum of peaks) | display MiB |",
                  "|---|---|---|---|---|---|---|---|---|---|---|---|---|"]
        for system in ("easi-run", "easi-run-h264", "baseline"):
            for fixture, row in summary.get(f"{system}/{mode}", {}).items():
                lines.append(
                    f"| {system} | {fixture} | {row['runs']} ({row['failed']}) | {fmt(row['presentedFps'])} "
                    f"| {fmt(row['kbitPerSecond'], 0)} | {fmt(row['mediaBytesPerSecond'], 0)} "
                    f"| {fmt(row['captureToDrawP50'], 0)}/{fmt(row['captureToDrawP95'], 0)} "
                    f"| {fmt(row['inputToDrawP50'], 0)}/{fmt(row['inputToDrawP95'], 0)} | {fmt(row['cpuCores'], 2)} "
                    f"| {fmt(row['fpsPerCpu'])} | {fmt(row['ssim'], 4)} "
                    f"| {fmt(row['memoryMiB'], 0)} ({fmt(row['memoryPeakMiB'], 0)}) | {fmt(row['displayMemoryPeakMiB'])} |")
        lines.append("")
    return "\n".join(lines)


def machine(runs):
    cpu = next((line.split(":", 1)[1].strip() for line in open("/proc/cpuinfo") if line.startswith("model name")), "?")
    # The browser of the images the runs actually measured, as each says itself.
    measured = sorted({(r["system"], r["browserImage"]) for r in runs if r.get("browserImage")})
    browsers = "; ".join(f"{system}: {browser_version(image)} ({image[7:19]})" for system, image in measured)
    return {
        "cpu": f"{cpu} ({os.cpu_count()} logical)",
        "gpu": "none (software rendering on both systems and the client)",
        "os": f"{platform.freedesktop_os_release().get('PRETTY_NAME')} {platform.release()}",
        "docker": docker("version", "-f", "{{.Server.Version}}"),
        "client": f"Playwright {PLAYWRIGHT.split('@')[0].split(':')[1]} Chromium",
        "browser": f"{browsers}; kiosk mode on both",
    }


def browser_version(image):
    """The browser an image runs, as the browser itself says."""
    return docker("run", "--rm", "--entrypoint", "/opt/google/chrome/chrome", image, "--version")


def controller_lanes():
    """The CONTROLLER_BENCH_LANES the running controller was started with."""
    env = docker("inspect", "-f", "{{range .Config.Env}}{{println .}}{{end}}", CONTROLLER).splitlines()
    return next((line.split("=", 1)[1] for line in env if line.startswith("CONTROLLER_BENCH_LANES=")), "")


def host_state(lane):
    """The host's load and, inside a lane, its CPUs' current frequency (MHz), at the start and end of each window."""
    state = {"loadavg": float(pathlib.Path("/proc/loadavg").read_text().split()[0])}
    if lane is not None:
        try:
            cpus = lanes.cpu_list(lane.baseline)
            freq = [int(pathlib.Path(f"/sys/devices/system/cpu/cpu{c}/cpufreq/scaling_cur_freq").read_text())
                    for c in cpus]
            state["laneMhz"] = round(statistics.mean(freq) / 1000)
        except OSError:
            pass  # a host without cpufreq reports no frequency
    return state


def plan(args):
    """Every run, in blocks of one fixture and repetition whose systems are shuffled, so no system keeps a lane or a
    position in the order."""
    shuffle = random.Random(args.seed)
    runs = []
    for fixture in args.fixtures.split(","):
        for rep in range(args.reps + args.headless_reps):
            block = [s for s in args.systems.split(",") if s != "easi-run-h264" or fixture in MOTION]
            shuffle.shuffle(block)
            runs += [(system, fixture, rep, rep < args.reps) for system in block]
    return runs


def run_one(admin, args, out, item, lane):
    system, fixture, rep, headed = item
    name = f"{system}-{fixture}-{'headed' if headed else 'headless'}-{rep}"
    frames = out / "frames" / name
    try:
        if system.startswith("easi-run"):
            codec = "h264" if system == "easi-run-h264" else "auto"
            result = run_easi_run(admin, fixture, args.seconds, headed, frames, lane, codec, args.bitrate)
            result["system"] = system
        else:
            result = run_baseline(fixture, args.seconds, headed, frames, lane)
    except Exception as e:  # one failed run is recorded and the benchmark goes on
        result = {"system": system, "fixture": fixture, "headed": headed, "error": str(e)[-2000:]}
    result |= {"rep": rep, "lane": None if lane is None else lane.index, "framesDir": str(frames)}
    return name, result


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0],
                                     formatter_class=argparse.ArgumentDefaultsHelpFormatter)
    parser.add_argument("--out", required=True, type=pathlib.Path, help="the results directory")
    parser.add_argument("--reps", type=int, default=5, help="headed repetitions per system and fixture")
    parser.add_argument("--headless-reps", type=int, default=5, help="headless repetitions per system and fixture")
    parser.add_argument("--seconds", type=int, default=60, help="measured seconds per run, after the warm-up")
    parser.add_argument("--fixtures", default=",".join(FIXTURES), help="comma-separated fixtures")
    parser.add_argument("--systems", default="easi-run,baseline", help="comma-separated systems (easi-run-h264 needs a display built with the h264 feature)")
    parser.add_argument("--lanes", type=int, default=0, help="runs measured at once, each in its own CPU lane")
    parser.add_argument("--bitrate", type=int, help="easi.run's video bitrate in kbit/s (default: the session's)")
    parser.add_argument("--seed", type=int, default=0, help="the shuffle of each block's systems")
    args = parser.parse_args()
    lane_set = lanes.layout(args.lanes) if args.lanes > 0 else []
    # The layout is the same for the first lanes of any count, so the controller may declare more than this run uses.
    wanted = lanes.controller_env(lane_set).split()
    if lane_set and controller_lanes().split()[:len(wanted)] != wanted:
        raise SystemExit(f"start the bench stack with BENCH_LANES='{lanes.controller_env(lane_set)}' (or more lanes)")
    out = args.out.resolve()
    if any((out / name).exists() for name in ("measured.jsonl", "runs.jsonl", "runs.jsonl.gz")):
        raise SystemExit(f"{out} already holds results; measure into a new directory")
    out.mkdir(parents=True, exist_ok=True)
    if "baseline" in args.systems:
        PINNED["baseline"] = docker("image", "inspect", "-f", "{{.Id}}", BASELINE_IMAGE)
    admin = Admin(os.environ["ADMIN_PASSWORD"]) if "easi-run" in args.systems else None
    if admin:
        admin.clear()
    free = queue.Queue()
    for lane in lane_set or [None]:
        free.put(lane)

    def in_a_lane(item):
        lane = free.get()
        try:
            return run_one(admin, args, out, item, lane)
        finally:
            free.put(lane)

    runs = []
    # Each run is logged as it ends, so an interrupted benchmark keeps what it measured; runs.jsonl follows the SSIM.
    with open(out / "measured.jsonl", "a") as log, \
            concurrent.futures.ThreadPoolExecutor(max_workers=max(1, len(lane_set))) as pool:
        for future in concurrent.futures.as_completed([pool.submit(in_a_lane, item) for item in plan(args)]):
            name, result = future.result()
            runs.append(result)
            log.write(json.dumps(result) + "\n")
            log.flush()
            brief = {k: result.get(k) for k in ("presentedFps", "kbitPerSecond", "cpuCores", "lane", "error")}
            print(name, json.dumps(brief), flush=True)
    # Quality frames are compared after every measurement, so the comparison never competes with one.
    for result in runs:
        if result["fixture"] in reference.FRAMES and "error" not in result:
            result["ssim"] = ssim(pathlib.Path(result["framesDir"]), result["fixture"], result["frames"])
    (out / "runs.jsonl").write_text("".join(json.dumps(r) + "\n" for r in runs))
    (out / "measured.jsonl").unlink()  # runs.jsonl now holds every run
    info = machine(runs) | {"lanes": len(lane_set), "seed": args.seed}
    summary = summarize(runs)
    (out / "summary.json").write_text(json.dumps({"machine": info, "summary": summary}, indent=2))
    (out / "summary.md").write_text(markdown(summary, info))
    print(markdown(summary, info))


if __name__ == "__main__":
    main()
