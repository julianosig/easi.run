#!/usr/bin/env python3
"""Builds the Rust tests a plan of scripts/affected.py selects, then runs every test binary at the same time.

    python3 scripts/affected.py | python3 scripts/cargo-test.py      # what scripts/check.sh does with its plan

From the plan's lines:

    crates     every test target of those crates (unit and integration tests), and the libraries' doctests, as
               `cargo test -p CRATE...` runs them
    stackless  display-tests' unit tests and the integration targets the line names, as
               `cargo test -p display-tests --lib --bins ...` runs them

Each binary runs as `cargo test` runs it: in its crate's directory, with the environment cargo gives it. Its output is
printed whole when it ends, and the run fails if any binary or the doctests fail.
"""

import json
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CARGO = os.environ.get("CARGO") or shutil.which("cargo") or "cargo"


class Stopped(Exception):
    """A signal to stop, with the binaries still running."""


def stop(signum, _frame):
    raise Stopped(signal.Signals(signum).name)


def plan():
    lines = {}
    for line in sys.stdin.read().splitlines():
        kind, _, rest = line.partition(" ")
        lines[kind] = rest.split()
    return lines


def metadata():
    """The workspace's packages by manifest path, and its target directory, from cargo metadata."""
    out = subprocess.run([CARGO, "metadata", "--locked", "--no-deps", "--format-version", "1"], cwd=ROOT, check=True,
                         stdout=subprocess.PIPE, text=True).stdout
    data = json.loads(out)
    return {p["manifest_path"]: p for p in data["packages"]}, Path(data["target_directory"])


def build(args, target_dir, natives):
    """Builds the tests `cargo test ARGS` would run, and adds to natives the library directories inside the target
    directory that its build scripts name. Returns each test binary as (manifest path, target, path, what its package's
    build script gives it when it runs: the environment that script set, with OUT_DIR)."""
    command = [CARGO, "test", "--locked", "--no-run", "--message-format=json-render-diagnostics", *args]
    print("+ " + " ".join(["cargo", *command[1:]]), file=sys.stderr, flush=True)
    process = subprocess.Popen(command, cwd=ROOT, stdout=subprocess.PIPE, text=True)
    artifacts, scripts = [], {}
    try:
        for line in process.stdout:
            message = json.loads(line)
            if message.get("reason") == "compiler-artifact" and message["profile"]["test"] and message["executable"]:
                artifacts.append(message)
            elif message.get("reason") == "build-script-executed":
                paths = (p.partition("=")[2] if "=" in p else p for p in message["linked_paths"])
                natives.update(p for p in paths if Path(p).is_relative_to(target_dir))
                scripts[message["package_id"]] = {**dict(message["env"]), "OUT_DIR": message["out_dir"]}
    except Stopped:
        process.kill()
        raise
    if process.wait() != 0:
        sys.exit(f"cargo-test.py: cargo test --no-run {' '.join(args)} failed")
    return [(m["manifest_path"], m["target"], m["executable"], scripts.get(m["package_id"], {})) for m in artifacts]


def environment(package, path, script, natives, sysroot_libdir):
    """What cargo sets for a test binary of this package: the library path (the build scripts' library directories
    inside the target directory, the profile's and its deps directories, the toolchain's, then the caller's), the
    package's build script's environment, cargo's own path, and the package's variables."""
    deps = Path(path).parent
    dirs = [*sorted(natives), str(deps.parent), str(deps), sysroot_libdir]
    dirs += [d for d in os.environ.get("LD_LIBRARY_PATH", "").split(os.pathsep) if d]
    version = package["version"]
    core, _, pre = version.partition("-")
    major, minor, patch = core.split(".")[:3]
    values = {
        "LD_LIBRARY_PATH": os.pathsep.join(dirs),
        **script,
        "CARGO": CARGO,
        "CARGO_MANIFEST_DIR": str(Path(package["manifest_path"]).parent),
        "CARGO_MANIFEST_PATH": package["manifest_path"],
        "CARGO_PKG_VERSION": version,
        "CARGO_PKG_VERSION_MAJOR": major,
        "CARGO_PKG_VERSION_MINOR": minor,
        "CARGO_PKG_VERSION_PATCH": patch.split("+")[0],
        "CARGO_PKG_VERSION_PRE": pre.split("+")[0],
        "CARGO_PKG_NAME": package["name"],
        "CARGO_PKG_AUTHORS": ":".join(package["authors"]),
        "CARGO_PKG_DESCRIPTION": package["description"] or "",
        "CARGO_PKG_HOMEPAGE": package["homepage"] or "",
        "CARGO_PKG_REPOSITORY": package["repository"] or "",
        "CARGO_PKG_LICENSE": package["license"] or "",
        "CARGO_PKG_LICENSE_FILE": package["license_file"] or "",
        "CARGO_PKG_RUST_VERSION": package["rust_version"] or "",
        "CARGO_PKG_README": package["readme"] or "",
    }
    return {**os.environ, **values}


def report(name, verdict, start, output):
    output.seek(0)
    text = output.read().decode(errors="replace")
    print(f"\n     Running {name}: {verdict} in {time.monotonic() - start:.1f} s\n{text}", end="", flush=True)


def run(runs):
    """Runs every (name, command, directory, environment) at once and prints each one's output whole when it ends.
    Returns the names of those that failed."""
    running, failed = {}, []
    try:
        for name, command, cwd, env in runs:
            output = tempfile.TemporaryFile()
            # In a process group of its own, which a stop ends whole, the processes a test started included.
            process = subprocess.Popen(command, cwd=cwd, env=env, stdin=subprocess.DEVNULL, stdout=output,
                                       stderr=subprocess.STDOUT, process_group=0)
            running[process.pid] = (name, process, output, time.monotonic())
        while running:
            pid, status = os.wait()
            if pid not in running:
                continue
            name, process, output, start = running.pop(pid)
            process.returncode = os.waitstatus_to_exitcode(status)
            if process.returncode != 0:
                failed.append(name)
            report(name, "ok" if process.returncode == 0 else f"FAILED (exit status {process.returncode})", start,
                   output)
    except Stopped as reason:
        # Stopped (a timeout, say): what each binary still running had printed, then the binary stops. One that ended
        # just as the signal came may be gone already.
        signal.signal(signal.SIGINT, signal.SIG_IGN)
        signal.signal(signal.SIGTERM, signal.SIG_IGN)
        for name, process, output, start in running.values():
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait()
            report(name, f"still running when stopped ({reason})", start, output)
        sys.exit("cargo-test.py: stopped with these still running:\n" +
                 "".join(f"  {name}\n" for name, *_ in running.values()))
    return failed


def main():
    # Both stop it, SIGINT too: a shell starts a background command with SIGINT ignored, and the binaries do not get
    # the terminal's, as each runs in a process group of its own.
    signal.signal(signal.SIGINT, stop)
    signal.signal(signal.SIGTERM, stop)
    lines = plan()
    crates = set(lines.get("crates", ()))
    stackless = lines.get("stackless")
    if not crates and stackless is None:
        return
    by_manifest, target_dir = metadata()
    binaries, runs, natives = [], [], set()
    if crates:
        selection = [arg for crate in sorted(crates) for arg in ("-p", crate)]
        binaries += [b for b in build(selection, target_dir, natives) if by_manifest[b[0]]["name"] in crates]
        if any(t["doctest"] for p in by_manifest.values() if p["name"] in crates for t in p["targets"]):
            command = [CARGO, "test", "--locked", "--doc", *selection]
            runs.append(("doctests (" + " ".join(["cargo", *command[1:]]) + ")", command, ROOT, os.environ))
    if stackless is not None:
        binaries += build(["-p", "display-tests", "--lib", "--bins", *stackless], target_dir, natives)
    sysroot_libdir = subprocess.run(["rustc", "--print", "target-libdir"], cwd=ROOT, check=True,
                                    stdout=subprocess.PIPE, text=True).stdout.strip()
    for manifest, target, path, script in binaries:
        package, directory = by_manifest[manifest], Path(manifest).parent
        source = os.path.relpath(target["src_path"], directory)
        name = f"{package['name']} {'/'.join(target['kind'])} {target['name']} ({source})"
        runs.append((name, [path], directory, environment(package, path, script, natives, sysroot_libdir)))
    failed = run(runs)
    if failed:
        sys.exit("cargo-test.py: failed:\n" + "".join(f"  {name}\n" for name in sorted(failed)))


if __name__ == "__main__":
    try:
        main()
    except Stopped as reason:
        sys.exit(f"cargo-test.py: stopped ({reason}) while building the tests")
