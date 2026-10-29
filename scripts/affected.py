#!/usr/bin/env python3
"""Selects the checks and images a change needs, from the files it touches.

    scripts/affected.py            # the working tree (staged, unstaged, untracked) against HEAD
    scripts/affected.py BASE       # everything since BASE (a commit, branch, or tag), plus the working tree
    scripts/affected.py --all      # every check and image

Prints one line per kind, each followed by what it selects (nothing when the line is absent):

    crates     the workspace crates to test with `cargo test -p`: a changed crate, every crate depending on it, and
               every crate whose sources read a changed file of another crate (`include_str!`, or a module's `#[path]`)
    stackless  display-tests without the stack: its lints and unit tests, and the integration targets it names
               (`--test NAME`): `hostile`, the hostile Wayland clients, which run the display binary, and `seccomp`,
               the browser seccomp profile's rules. It names none when only an end-to-end test file changed
    client     the client tests (`npm test` in client/)
    site       the landing and docs pages' check and build (`npm run build` in site/), and the docs' tool list
               against admin/src/tools.json (`scripts/tools.py --site`)
    licenses   the license report (scripts/licenses.py)
    fuzz       the fuzz targets' lints (`cargo clippy` of tests/fuzz, a separate workspace that tests/fuzz/run.sh runs)
    demo       the public demo's host lists (scripts/demo.py)
    scripts    the shell and Python scripts' lints (ShellCheck and Ruff), for every `*.sh` and `*.py` not ignored
    installer  install.sh's tests against stubbed system commands (scripts/install-test.sh)
    pins       the pinned versions' consistency (scripts/pins.py, whose docstring lists what it checks)
    vendored   third_party/'s crate against its release and patch (scripts/third_party.sh)
    images     the images to rebuild before end-to-end tests
    e2e        the end-to-end test targets to run against the rebuilt stack (`--test NAME`), found by cargo metadata

Unknown paths select everything: a file this script cannot place could affect anything.
"""

import json
import re
import subprocess
import sys
from pathlib import Path

from pins import DISPLAY_MANIFEST, FUZZ_MANIFEST, INSTALLER as ROOT_IMAGE_FILE, PINNED, PLAYWRIGHT_NPM

ROOT = Path(__file__).resolve().parent.parent

# Paths that change no build, test, or image: these prefixes, and the files in INERT_FILES (`scripts/e2e.sh` runs the
# end-to-end tests, and nothing reads it; the root `.gitignore` feeds no build, `.gitleaks.toml` only the secret scan,
# and `.dockerignore` is in RUST_WIDE).
INERT = ("docs/", ".github/", "bench/results/", "bench/runner/")
INERT_FILES = {"scripts/e2e.sh", ".gitignore", ".gitleaks.toml"}
# Paths of client/ that its tests read and its build (client/dist, what the admin image serves) does not.
CLIENT_ONLY = ("client/test/", "client/.gitignore")
# Paths an image copies that no end-to-end test exercises: the web client, which the admin image serves and no test in
# KNOWN_E2E loads (they reach the admin through /api, /ws and /mcp only). A change to it rebuilds the admin image and
# runs no end-to-end test but one KNOWN_E2E does not list yet, which every image's change selects.
E2E_BLIND = ("client/", "i18n/")
# Scripts an end-to-end test runs, with those tests: `scripts/tools.py --check` compares the admin's tool snapshot with
# the agent image's driver.
E2E_SCRIPTS = {"scripts/tools.py": {"agents"}}
# Paths every Rust crate builds with.
RUST_WIDE = ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "rustfmt.toml", ".dockerignore")
# The checker itself: a change to it re-runs everything.
SELF = ("scripts/affected.py", "scripts/cargo-test.py", "scripts/check.sh")
# The patched dependency (third_party/README.md): a change to any of it re-runs everything.
VENDORED = "third_party/"
# The fuzz targets' workspace (tests/fuzz/run.sh), which cargo metadata of this workspace does not list.
FUZZ = ("display-fuzz", "tests/fuzz/")
# How a Rust source reads another file at build time: `include_str!`/`include_bytes!`, or a module's `#[path]`. A
# test that reads another crate's source declares it this way, and select() runs that test when the source changes.
SOURCE_READ = re.compile(r"\binclude_(?:str|bytes)\s*!\s*[(\[{]\s*|#\s*\[\s*path\s*=\s*")
# The path such a read names: a plain string literal without escapes, or a raw one (`r"..."`, `r#"..."#`).
LITERAL = re.compile(r'"([^"\\]*)"|r(#*)"(.*?)"\2', re.S)
# A reader whose read names its file some other way (a macro, an escape): it is selected by every change.
ANYWHERE = "*"
# The installer and its tests: no crate, image or end-to-end test reads them.
INSTALLER = ("install.sh", "scripts/install-test.sh")

# The crates the root Dockerfile's shared stage builds together: their features unify, so a change to any of them can
# change each binary that stage makes.
RUST_BUILD = ["admin", "agent", "browser-launch", "controller", "proxy"]
# The root Dockerfile, which holds every image's target but the display's and the fixture host's: a change to it can
# change each of them.
DOCKERFILE = "Dockerfile"
# The notices every published image carries (scripts/notices.py writes them, with LICENSE and scripts/notices/).
NOTICES = ["scripts/notices.py", "scripts/notices/", "LICENSE"]
# Each image: the crates whose binaries it ships, or whose build makes them, and the other paths it copies in.
IMAGES = {
    "display": (
        ["display"],
        ["display/", "scripts/svt-av1.sh", "scripts/opus.sh", "scripts/static-library.sh", *NOTICES],
    ),
    "admin": (RUST_BUILD, [DOCKERFILE, "admin/", "client/", "i18n/", *NOTICES]),
    "controller": (RUST_BUILD, [DOCKERFILE, "controller/", "browser/seccomp.json", *NOTICES]),
    "proxy": (RUST_BUILD, [DOCKERFILE, "proxy/", *NOTICES]),
    "browser": (RUST_BUILD, [DOCKERFILE, "browser/", *NOTICES]),
    "agent": (RUST_BUILD, [DOCKERFILE, "agent/", *NOTICES]),
    "fixtures": (["bench"], ["bench/Dockerfile", "bench/fixtures/", "bench/src/"]),
    "baseline": ([], [DOCKERFILE, "bench/kasmvnc/", "browser/policy.json", "browser/launch/src/chrome-flags"]),
    # ffmpeg, which the stream tests decode with; not a compose service (scripts/e2e.sh builds it).
    "tools": ([], ["bench/tools/"]),
}
# The files whose pinned versions the license report names: the scripts that build the display's encoders, and the
# root Dockerfile, whose agent target pins the Cua Driver release.
REPORT_PINS = ("scripts/svt-av1.sh", "scripts/opus.sh", DOCKERFILE)
# Paths (or directory prefixes) outside a crate directory that its build reads, with the crates reading them.
NATIVE_INPUTS = {
    "scripts/svt-av1.sh": {"display"},
    "scripts/opus.sh": {"display"},
    "scripts/static-library.sh": {"display"},
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
    "profiles": SESSION,
    "udp": SESSION,
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


def source_reads(dirs):
    """Each file a crate's sources read from outside the crate's own directory (SOURCE_READ), with the crates (or the
    fuzz workspace) reading it."""
    reads = {}
    for name, d in [*dirs.items(), FUZZ]:
        base = ROOT / d
        sources = [base / "build.rs"]
        sources += [s for inputs in ("src", "tests", "fuzz_targets") for s in (base / inputs).rglob("*.rs")]
        for source in sources:
            if not source.is_file():
                continue
            text = source.read_text()
            for read in SOURCE_READ.finditer(text):
                literal = LITERAL.match(text, read.end())
                if literal is None:
                    reads.setdefault(ANYWHERE, set()).add(name)
                    continue
                target = (source.parent / (literal[1] if literal[1] is not None else literal[3])).resolve()
                if not target.is_relative_to(base):
                    key = target.relative_to(ROOT).as_posix() if target.is_relative_to(ROOT) else ANYWHERE
                    reads.setdefault(key, set()).add(name)
    return reads


def closure(crates, dependents):
    todo, seen = list(crates), set(crates)
    while todo:
        for user in dependents.get(todo.pop(), ()):
            if user not in seen:
                seen.add(user)
                todo.append(user)
    return seen


COMPOSE = ("compose.yaml", "compose.bench.yaml")
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
    reads = source_reads(dirs)
    crates, copied, client, licenses, site, fuzz, demo = set(), set(), False, False, False, False, False
    scripted, pins, vendored, scripts, installer = set(), False, False, False, False
    for f in files:
        # The vendored crate, its README and changelog included: scripts/third_party.sh compares all of it with its
        # release.
        if f in SELF or f in RUST_WIDE or f.startswith(VENDORED):
            everything = True
            break
        # Before any rule below skips the file: the crates (or the fuzz targets) whose sources read it.
        declared = reads.get(f, set()) | reads.get(ANYWHERE, set())
        fuzz |= FUZZ[0] in declared
        crates |= declared - {FUZZ[0]}
        # Before any rule below skips the file: a script lints the same wherever it lives.
        scripts |= f.endswith((".sh", ".py"))
        # The docs pages list the pinned tools, which `scripts/tools.py --site` compares with the snapshot.
        site |= f in ("admin/src/tools.json", "scripts/tools.py")
        # Every Dockerfile's base images and the other files pinning an image by digest (scripts/pins.py).
        pins |= Path(f).name == "Dockerfile" or f in PINNED or f in PLAYWRIGHT_NPM or f == ROOT_IMAGE_FILE
        # The display's dependency versions, which the fuzz targets' manifest repeats.
        pins |= f in (FUZZ_MANIFEST, DISPLAY_MANIFEST)
        if f in E2E_SCRIPTS:
            scripted |= E2E_SCRIPTS[f]
            continue
        if f in INSTALLER:
            installer = True
            continue
        if f.startswith("tests/fuzz/"):
            fuzz = True  # a separate workspace, built here and run by tests/fuzz/run.sh
            continue
        if f.startswith("demo/") or f == "scripts/demo.py":
            # The public demo's own installation (its Compose file, walled-garden pages and certificate script): it
            # builds the same images from the same sources, and no image or test reads it. Besides scripts/demo.py,
            # only CI's images job reads it (its Compose and nginx check), which check.sh names as a next step.
            demo = True
            continue
        demo |= f == "client/src/ui/demos.ts"
        if f == "scripts/third_party.sh":
            vendored = True  # any change under third_party/ itself selects everything (VENDORED), this check included
            continue
        if f == "scripts/pins.py":
            pins = True  # reads the Dockerfiles, PINNED, the toolchain files and two manifests; only affected.py reads it
            continue
        # check.sh runs compare.py and settings.py on every plan, run.sh only starts the development image, no check
        # runs coverage.sh or the coverage.py it calls, and pin.py (which only the owner runs, to pin install.sh to a
        # release) reads the registry: their lints are all they need.
        if f in ("scripts/compare.py", "scripts/settings.py", "scripts/dev/run.sh", "scripts/coverage.sh",
                 "scripts/coverage.py", "scripts/pin.py"):
            continue
        # The generated report itself (a hand edit fails the check that regenerates it), and the SBOMs' script.
        if f.startswith("docs/sbom/") or f in ("docs/licenses.md", "scripts/licenses.py", "scripts/sbom.sh"):
            licenses = True
            continue
        # The license report checks the notices (scripts/notices.py, with its texts, Markdown ones too), which the
        # images copy in.
        if f.startswith("scripts/notices"):
            licenses = True
            copied.add(f)
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
        # The crate owning the file, the crates whose build reads it from outside, and those whose sources read it.
        readers = ({crate} if crate else set()) | {c for p, cs in NATIVE_INPUTS.items() if f.startswith(p) for c in cs}
        readers |= reads.get(f, set()) - {FUZZ[0]}
        # A crate manifest can move a dependency between build-only and runtime without touching Cargo.lock.
        licenses |= crate is not None and f == dirs.get(crate, "") + "Cargo.toml"
        client |= f.startswith(("client/", "i18n/", "protocol/vectors/"))
        site |= f.startswith("i18n/")  # the languages and formatting the site shares with the client
        licenses |= f == "client/package-lock.json"
        licenses |= f in REPORT_PINS  # the report names the versions they pin
        in_image = f in COMPOSE or any(f.startswith(p) for _, extra in IMAGES.values() for p in extra)
        if not readers and not in_image:
            everything = True
            break
        crates |= readers
        copied.add(f)
    if everything:
        return {"crates": sorted(set(dirs) - {"display-tests"}), "stackless": sorted(STACKLESS), "client": True,
                "site": True,
                "licenses": True, "fuzz": True, "demo": True, "scripts": True, "installer": True, "pins": True,
                "vendored": True,
                "images": sorted(IMAGES),
                "e2e": sorted(e2e_targets)}
    crates = closure(crates, dependents)

    def rebuilt(paths):
        return {name for name, (bins, extra) in IMAGES.items()
                if crates & set(bins) or any(f.startswith(p) for f in paths for p in extra)}

    images = rebuilt(copied)
    exercised = rebuilt({f for f in copied if not f.startswith(E2E_BLIND)})
    e2e = {name for name, (_, uses) in e2e_targets.items() if uses & (exercised if name in KNOWN_E2E else images)}
    e2e |= scripted & set(e2e_targets)
    shared = any(f.startswith(("tests/src/", "tests/Cargo.toml")) for f in copied)
    for f in copied:
        if f.startswith("tests/tests/"):
            # The target built from this file; any other file there is shared by every target.
            owners = {name for name, (source, _) in e2e_targets.items() if f == source}
            e2e |= owners if owners or Path(f).stem in STACKLESS else set(e2e_targets)
    if shared or set(COMPOSE) & copied:
        e2e = set(e2e_targets)
    if set(COMPOSE) & copied:
        # A Compose file can change any image it builds (its build arguments, its Dockerfile), and `up` rebuilds none
        # that exists: every one of them is rebuilt, which BuildKit's cache makes quick when nothing changed.
        images |= set(IMAGES) - {"tools"}
    stackless = {name for name, (inputs, paths) in STACKLESS.items()
                 if crates & inputs or any(f.startswith(paths) or f == f"tests/tests/{name}.rs" for f in copied)}
    if shared:
        stackless = set(STACKLESS)
    # Any other test file (an end-to-end one, or `common/`) still needs display-tests' lints.
    stackless = sorted(stackless) or any(f.startswith("tests/tests/") for f in copied)
    return {"crates": sorted(crates - {"display-tests"}), "stackless": stackless, "client": client,
            "site": site, "licenses": licenses, "fuzz": fuzz or "display-protocol" in crates,
            "demo": demo, "scripts": scripts, "installer": installer, "pins": pins, "vendored": vendored,
            "images": sorted(images), "e2e": sorted(e2e)}


def main():
    args = sys.argv[1:]
    everything = args == ["--all"]
    base = None if everything or not args else args[0]
    plan = select([] if everything else changed(base), everything)
    for kind, value in plan.items():
        if value is True:
            print(kind)
        elif value:
            print(kind, *[arg for v in value for arg in (("--test", v) if kind in ("stackless", "e2e") else (v,))])


if __name__ == "__main__":
    main()
