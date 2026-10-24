#!/usr/bin/env python3
"""Selects the checks and images a change needs, from the files it touches.

    scripts/affected.py            # the working tree (staged, unstaged, untracked) against HEAD
    scripts/affected.py BASE       # everything since BASE (a commit, branch, or tag), plus the working tree
    scripts/affected.py --all      # every check and image

Prints one line per kind, each followed by what it selects (nothing when the line is absent):

    crates     the workspace crates to test with `cargo test -p` (a changed crate and every crate depending on it)
    stackless  display-tests without the stack: its lints and unit tests, and the integration targets it names
               (`--test NAME`): `hostile`, the hostile Wayland clients, which run the display binary, and `seccomp`,
               the browser seccomp profile's rules. It names none when only an end-to-end test file changed
    client     the client tests (`npm test` in client/)
    site       the landing and docs pages' check and build (`npm run build` in site/)
    licenses   the license report (scripts/licenses.py)
    fuzz       the fuzz targets' lints (`cargo clippy` of tests/fuzz, a separate workspace that tests/fuzz/run.sh runs)
    demo       the public demo's host lists (scripts/demo.py)
    scripts    the shell and Python scripts' lints (ShellCheck and Ruff), for every tracked `*.sh` and `*.py`
    pins       the base images' digests, the Rust version, and the Google Chrome package, each pinned one way
               across the Dockerfiles, rust-toolchain.toml, and Cargo.toml (scripts/pins.py)
    vendored   third_party/'s crate against its release and patch (scripts/third_party.sh)
    compose    the egress overlay merged onto compose.yaml (`docker compose config`), which no end-to-end test loads
    images     the images to rebuild before end-to-end tests
    e2e        the end-to-end test targets to run against the rebuilt stack (`--test NAME`), found by cargo metadata

Unknown paths select everything: a file this script cannot place could affect anything.
"""

import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# Paths that change no build, test, or image: these prefixes, and the files in INERT_FILES (`scripts/e2e.sh` runs the
# end-to-end tests, and nothing reads it; the root `.gitignore` feeds no build, and `.dockerignore` is in RUST_WIDE).
INERT = ("docs/", ".github/", "bench/results/", "bench/runner/")
INERT_FILES = {"LICENSE", "scripts/e2e.sh", ".gitignore"}
# Paths of client/ that its tests read and its build (client/dist, what the admin image serves) does not.
CLIENT_ONLY = ("client/test/", "client/.gitignore")
# Scripts an end-to-end test runs, with those tests: `scripts/tools.py --check` compares the admin's tool snapshot with
# the agent image's driver.
E2E_SCRIPTS = {"scripts/tools.py": {"agents"}}
# Paths every Rust crate builds with.
RUST_WIDE = ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "rustfmt.toml", ".dockerignore")
# The checker itself: a change to it re-runs everything.
SELF = ("scripts/affected.py", "scripts/check.sh")
# The patched dependency (third_party/README.md): a change to any of it re-runs everything.
VENDORED = "third_party/"
# The display's sources the fuzz targets include (its recording's camera and microphone decoders).
FUZZED = ("display/src/stream/camera.rs", "display/src/stream/opus.rs")

# Each image: the crates whose binaries it ships, and the other paths it copies in.
IMAGES = {
    "display": (["display"], ["display/", "scripts/svt-av1.sh", "scripts/opus.sh"]),
    "admin": (["admin"], ["admin/", "client/", "protocol/vectors/"]),
    "controller": (["controller"], ["controller/", "browser/seccomp.json"]),
    "proxy": (["proxy"], ["proxy/"]),
    "browser": (["browser-launch"], ["browser/"]),
    "agent": (["agent"], ["agent/"]),
    "fixtures": (["bench"], ["bench/Dockerfile", "bench/fixtures/", "bench/src/"]),
    "baseline": ([], ["bench/kasmvnc/", "browser/policy.json"]),
    # ffmpeg, which the stream tests decode with; not a compose service (scripts/e2e.sh builds it).
    "tools": ([], ["bench/tools/"]),
}
# The files whose pinned versions the license report names: the scripts that build the display's encoders, and the
# agent image's Dockerfile, which pins the Cua Driver release.
ENCODER_SCRIPTS = ("scripts/svt-av1.sh", "scripts/opus.sh", "agent/Dockerfile")
# Paths (or directory prefixes) outside a crate directory that its build reads, with the crates reading them.
NATIVE_INPUTS = {
    "scripts/svt-av1.sh": {"display"},
    "scripts/opus.sh": {"display"},
    # The easi_files_v1 Wayland protocol, which both ends generate their code from.
    "protocol/wayland/": {"display", "browser-launch"},
}
# Each end-to-end test file: the images whose behavior it exercises. Every session runs the display, browser, proxy,
# and agent images through the admin and the controller, against the fixture host.
SESSION = {"display", "admin", "browser", "proxy", "agent", "controller", "fixtures"}
KNOWN_E2E = {
    "stream": SESSION | {"tools"},
    "audio": SESSION | {"tools"},
    "files": SESSION,
    "features": SESSION,
    "control": SESSION,
    "agents": SESSION,
    # The security probes run in the pinned Debian image, and the egress test's upstream is a proxy image.
    "security": SESSION,
}
# The integration tests that need no running stack (the end-to-end ones are ignored unless run with --ignored), each
# with the crates and the paths whose change runs them.
STACKLESS = {
    "hostile": ({"display"}, ()),
    "seccomp": (set(), ("browser/seccomp.json",)),
}


def changed(base):
    def lines(*args):
        out = subprocess.run(["git", *args], cwd=ROOT, check=True, capture_output=True, text=True).stdout
        return [line for line in out.splitlines() if line]

    # Without rename detection a move lists both paths, so the source of a move is classified too.
    files = set(lines("diff", "--name-only", "--no-renames", "HEAD"))
    files |= set(lines("ls-files", "--others", "--exclude-standard"))
    if base:
        files |= set(lines("diff", "--name-only", "--no-renames", f"{base}...HEAD"))
    return sorted(files)


def workspace():
    """Each crate's directory, each crate's reverse path dependencies, and the end-to-end test targets: every test
    target of display-tests that needs the stack, each with the images it exercises (all of them when unknown)."""
    meta = json.loads(subprocess.run(["cargo", "metadata", "--format-version", "1", "--no-deps", "--locked"],
                                     cwd=ROOT, check=True, capture_output=True, text=True).stdout)
    dirs, dependents, e2e = {}, {}, {}
    for package in meta["packages"]:
        if package["name"] == "display-tests":
            for target in package["targets"]:
                if "test" in target["kind"] and target["name"] not in STACKLESS:
                    source = Path(target["src_path"]).relative_to(ROOT).as_posix()
                    e2e[target["name"]] = (source, KNOWN_E2E.get(target["name"], set(IMAGES)))
        dirs[package["name"]] = Path(package["manifest_path"]).parent.relative_to(ROOT).as_posix() + "/"
        for dep in package["dependencies"]:
            if dep.get("path"):
                dependents.setdefault(dep["name"], set()).add(package["name"])
    return dirs, dependents, e2e


def closure(crates, dependents):
    todo, seen = list(crates), set(crates)
    while todo:
        for user in dependents.get(todo.pop(), ()):
            if user not in seen:
                seen.add(user)
                todo.append(user)
    return seen


COMPOSE = ("compose.yaml", "compose.bench.yaml", "bench/egress.json")
# The deployment's egress overlay: only its own check reads it, against the base file it extends.
OVERLAY = "compose.egress.yaml"
# What a crate's own build reads, relative to its directory.
CRATE_INPUTS = ("src/", "tests/", "vectors/", "build.rs", "Cargo.toml")


def owner(path, dirs):
    """The crate whose build reads `path`, if any."""
    for name, d in sorted(dirs.items(), key=lambda item: -len(item[1])):
        if path.startswith(d):
            return name if path[len(d):].startswith(CRATE_INPUTS) else None
    return None


def select(files, everything=False):
    dirs, dependents, e2e_targets = workspace()
    crates, copied, client, licenses, site, fuzz, demo = set(), set(), False, False, False, False, False
    scripted, pins, vendored, overlay, scripts = set(), False, False, False, False
    for f in files:
        # The vendored crate, its README and changelog included: scripts/third_party.sh compares all of it with its
        # release.
        if f in SELF or f in RUST_WIDE or f.startswith(VENDORED):
            everything = True
            break
        # Before any rule below skips the file: a script lints the same wherever it lives.
        scripts |= f.endswith((".sh", ".py"))
        # The docs pages list the pinned tools, which `scripts/tools.py --site` compares with the snapshot.
        site |= f in ("admin/src/tools.json", "scripts/tools.py")
        # Every Dockerfile's base images, and the two that install Google Chrome (scripts/pins.py).
        pins |= Path(f).name == "Dockerfile"
        overlay |= f == "compose.yaml"
        if f == OVERLAY:
            overlay = True
            continue
        if f in E2E_SCRIPTS:
            scripted |= E2E_SCRIPTS[f]
            continue
        if f.startswith("tests/fuzz/"):
            fuzz = True  # a separate workspace, built here and run by tests/fuzz/run.sh
            continue
        if f.startswith("demo/") or f == "scripts/demo.py":
            # The public demo's own installation (its Compose file, walled-garden pages and certificate script): it
            # builds the same images from the same sources, and no other check, image, or test reads it.
            demo = True
            continue
        demo |= f == "client/src/ui/demos.ts"
        if f == "scripts/third_party.sh":
            vendored = True  # any change under third_party/ itself selects everything (VENDORED), this check included
            continue
        if f == "scripts/pins.py":
            pins = True  # reads the Dockerfiles and the toolchain files, and nothing reads it
            continue
        # The generated report itself (a hand edit fails the check that regenerates it), and the SBOMs' script.
        if f.startswith("docs/sbom/") or f in ("docs/licenses.md", "scripts/licenses.py", "scripts/sbom.sh"):
            licenses = True
            continue
        if f.endswith(".md") or f.startswith(INERT) or f in INERT_FILES:
            continue
        if f.startswith("site/"):
            site = True  # the static pages: no crate, image, or end-to-end test reads them
            licenses |= f == "site/package-lock.json"
            continue
        if f.startswith(CLIENT_ONLY):
            client = True
            continue
        crate = owner(f, dirs)
        readers = {crate} if crate else {c for p, cs in NATIVE_INPUTS.items() if f.startswith(p) for c in cs}
        # A crate manifest can move a dependency between build-only and runtime without touching Cargo.lock.
        licenses |= crate is not None and f == dirs.get(crate, "") + "Cargo.toml"
        client |= f.startswith(("client/", "protocol/vectors/"))
        licenses |= f == "client/package-lock.json"
        licenses |= f in ENCODER_SCRIPTS  # the report names the versions they pin
        in_image = f in COMPOSE or any(f.startswith(p) for _, extra in IMAGES.values() for p in extra)
        if not readers and not in_image:
            everything = True
            break
        crates |= readers
        copied.add(f)
    if everything:
        return {"crates": sorted(set(dirs) - {"display-tests"}), "stackless": sorted(STACKLESS), "client": True,
                "site": True,
                "licenses": True, "fuzz": True, "demo": True, "scripts": True, "pins": True, "vendored": True,
                "compose": True,
                "images": sorted(IMAGES),
                "e2e": sorted(e2e_targets)}
    crates = closure(crates, dependents)
    images = {name for name, (bins, extra) in IMAGES.items()
              if crates & set(bins) or any(f.startswith(p) for f in copied for p in extra)}
    e2e = {name for name, (_, uses) in e2e_targets.items() if uses & images} | (scripted & set(e2e_targets))
    shared_tests = ("tests/src/", "tests/Cargo.toml")
    for f in copied:
        if f.startswith("tests/tests/"):
            # The target built from this file (a test file, or a file inside a `name/main.rs` test's directory);
            # any other file there is shared by every target.
            owners = {name for name, (source, _) in e2e_targets.items()
                      if f == source or (source.endswith("/main.rs") and f.startswith(source[:-len("main.rs")]))}
            e2e |= owners if owners or Path(f).stem in STACKLESS else set(e2e_targets)
    if any(f.startswith(shared_tests) for f in copied) or set(COMPOSE) & copied:
        e2e = set(e2e_targets)
    if set(COMPOSE) & copied:
        # A Compose file can change any image it builds (its build arguments, its Dockerfile), and `up` rebuilds none
        # that exists: every one of them is rebuilt, which BuildKit's cache makes quick when nothing changed.
        images |= set(IMAGES) - {"tools"}
    stackless = {name for name, (inputs, paths) in STACKLESS.items()
                 if crates & inputs or any(f.startswith(paths) or f == f"tests/tests/{name}.rs" for f in copied)}
    if any(f.startswith(shared_tests) for f in copied):
        stackless = set(STACKLESS)
    # Any other test file (an end-to-end one, or `common/`) still needs display-tests' lints.
    stackless = sorted(stackless) or any(f.startswith("tests/tests/") for f in copied)
    return {"crates": sorted(crates - {"display-tests"}), "stackless": stackless, "client": client, "site": site,
            "licenses": licenses, "fuzz": fuzz or "display-protocol" in crates or bool(set(FUZZED) & copied),
            "demo": demo, "scripts": scripts, "pins": pins, "vendored": vendored, "compose": overlay, "images": sorted(images),
            "e2e": sorted(e2e)}


def main():
    args = sys.argv[1:]
    everything = args == ["--all"]
    base = None if everything or not args else args[0]
    plan = select([] if everything else changed(base), everything)
    for kind, value in plan.items():
        if value is True:
            print(kind)
        elif value:
            print(kind, *value)


if __name__ == "__main__":
    main()
