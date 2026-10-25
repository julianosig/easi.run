#!/usr/bin/env python3
"""Writes the license and notice texts a published image owes for what is compiled or copied into it, and checks
that an image carries them.

    scripts/notices.py cargo OUT CRATE... [--built-with CRATE,...] [--features FEATURES]
                                             every third-party crate the crates' binaries link (normal dependencies,
                                             for Linux on x86-64, with the features of one build of these and the
                                             --built-with crates), from the Cargo workspace here, or from the one
                                             whose manifest CARGO_MANIFEST names
    scripts/notices.py npm OUT DIR           every production package in DIR's package-lock.json, read from
                                             DIR/node_modules
    scripts/notices.py source OUT [COMMIT]   OUT/LICENSE, the repository's, and OUT/SOURCE, which says where the
                                             source of an image built from COMMIT (or a working tree) is
    scripts/notices.py image IMAGE ROLE [COMMIT]
                                             checks that IMAGE carries /usr/share/doc/easi.run with what its role
                                             needs (REQUIRED), the repository's LICENSE among them, and a SOURCE that
                                             names COMMIT when given

OUT gets, for each package, its name, version, license expression and where its source is, then the license, copyright
and NOTICE files it ships (LICENSE*, LICENCE*, COPYING*, COPYRIGHT*, NOTICE*, UNLICENSE*, anywhere in it but its tests,
examples and documentation, so the licenses of the C sources or Unicode tables a crate bundles come too). A text
shared word for word by several packages is written once, under all of their names. A package whose files grant no
license with any text in them (a COPYRIGHT or NOTICE alone does not) fails the run, as does a license file cargo
points at and that is missing, unless scripts/notices/<name>-<version>/ holds the license its repository ships
(README.md there says where each is from), which is added beside the package's own notices. The crates are those
`cargo tree` resolves for the binaries' crates, with the features one build of them and the --built-with crates
unifies; a procedural macro and what only it depends on run at build time and are left out, and so is a path
dependency with no license of its own outside third_party/, which belongs to the workspace's repository and its
license. An empty license or notice file fails the run too. The cargo and npm
modes read only what the build already has: no network.
"""

import json
import os
import re
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TARGET = "x86_64-unknown-linux-gnu"
NAME = re.compile(r"^(licen[cs]e|copying|copyright|notice|unlicense)([-_.].*)?$", re.I)
# The files among them that grant a license, not only name a copyright holder or a NOTICE.
LICENSE = re.compile(r"^(licen[cs]e|copying|unlicense)([-_.].*)?$", re.I)
SKIP = re.compile(r"^(tests?|benches|examples?|docs?|fixtures?|testdata|\.git.*)$", re.I)
DOC = "usr/share/doc/easi.run"
FALLBACK = Path(__file__).resolve().parent / "notices"
REPOSITORY = "https://github.com/julianosig/easi.run"
# What each published image's notices directory holds, beside LICENSE and SOURCE.
REQUIRED = {
    "admin": ["rust-crates.md", "npm-packages.md"],
    "controller": ["rust-crates.md"],
    "proxy": ["rust-crates.md"],
    "browser": ["rust-crates.md"],
    "agent": ["rust-crates.md", "cua-driver/LICENSE.md", "cua-driver/THIRD_PARTY_NOTICES.md",
              "cua-driver/Inter-OFL.txt", "cua-driver/rust-crates.md"],
    "display": ["rust-crates.md", "svt-av1/LICENSE.md", "svt-av1/LICENSE-BSD2.md", "svt-av1/PATENTS.md",
                "svt-av1/third_party/fastfeat/LICENSE", "svt-av1/third_party/safestringlib/LICENSE",
                "opus/COPYING", "opus/AUTHORS"],
}


def license_files(directory):
    """The license and notice files anywhere under `directory` (tests, examples and documentation aside), as sorted
    relative paths: a crate's bundled sources, such as C libraries or Unicode tables, keep their own."""
    found = []
    for top, directories, files in os.walk(directory):
        directories[:] = sorted(d for d in directories if not SKIP.match(d) and d != "target")
        found += [os.path.relpath(os.path.join(top, f), directory) for f in files if NAME.match(f)]
    return sorted(found)


def license_bearing(directory, files):
    """Whether one of `files` is a license (not only a copyright or NOTICE file) with any text in it."""
    return any(LICENSE.match(Path(f).name) and (Path(directory) / f).is_file() and
               (Path(directory) / f).read_text(errors="replace").strip() for f in files)


def write(out, title, packages):
    """Writes the notices of `packages` ((name, version, license, directory, extra files, source)) to `out`; returns
    the names of those with no license text."""
    texts, missing = {}, []
    for name, version, expression, directory, extra, _ in sorted(packages):
        found = [(directory, f) for f in sorted(set(license_files(directory)) | set(extra))]
        fallback = FALLBACK / f"{name}-{version}"
        if not license_bearing(directory, [f for _, f in found]) and fallback.is_dir():
            # The package's own notices stay; its repository's license comes beside them.
            found += [(fallback, f) for f in license_files(fallback)]
        if not any(license_bearing(d, [f]) for d, f in found):
            missing.append(f"{name} {version}")
        for base, relative in found:
            path = Path(base) / relative
            if not path.is_file():
                missing.append(f"{name} {version}: {relative}")
                continue
            text = path.read_text(errors="replace").strip()
            if not text:
                missing.append(f"{name} {version}: {relative} is empty")
                continue
            # Written once for every package whose file has the same words, whatever its spacing.
            key = " ".join(text.split())
            texts.setdefault(key, (text, []))[1].append(f"{name} {version} ({expression}): {relative}")
    lines = [f"# {title}", "", f"{len(packages)} packages. Each text below is shipped by the packages listed above it.",
             "Each package's source, as published, is at the address beside it (where the Mozilla Public License, "
             "for one, requires it to be available).", ""]
    lines += ["## Packages", ""] + [f"- {n} {v}: {e or 'no license expression'}; source: {s}"
                                    for n, v, e, _, _, s in sorted(packages)] + [""]
    fence = "`" * 3
    for text, owners in sorted(texts.values(), key=lambda item: item[1]):
        lines += ["---", ""] + [f"- {owner}" for owner in owners]
        lines += ["", fence + "text", text.replace(fence, "'" * 3), fence, ""]
    Path(out).parent.mkdir(parents=True, exist_ok=True)
    Path(out).write_text("\n".join(lines))
    return missing


def report(text):
    """What is wrong with a written report: its count and package list agree, and every package is among the owners
    of a group whose text (a fence inside one is written as three quotes) has words in it."""
    fence = "`" * 3
    count = re.search(r"^(\d+) packages\.", text, re.M)
    listed = re.findall(r"^- (\S+ \S+): .*; source: \S.*$", text, re.M)
    problems = []
    if not text.startswith("# ") or not count or int(count.group(1)) != len(listed) or not listed:
        problems.append("its package count and list disagree, or it lists none")
    body = text.split("\n---\n", 1)[1] if "\n---\n" in text else ""
    group = re.compile(r"\n((?:- [^\n]+\n)+)\n" + fence + r"text\n((?:(?!" + fence + r").)*)\n" + fence + r"\n(?:\n---\n|$)",
                       re.S)
    owners, position = set(), 0
    while position < len(body):
        found = group.match(body, position)
        if not found:
            problems.append(f"it is malformed after its first {len(owners)} owners")
            break
        if not found.group(2).strip():
            problems.append(f"an empty text, for {found.group(1).split()[1:3]}")
        owners |= {" ".join(line.split()[1:3]) for line in found.group(1).splitlines()}
        position = found.end()
    problems += [f"no text for {p}" for p in listed if p not in owners]
    return problems


def cargo(out, crates, built, features):
    manifest = ["--manifest-path", os.environ["CARGO_MANIFEST"]] if os.environ.get("CARGO_MANIFEST") else []

    def run(*args):
        return subprocess.run(["cargo", *args, "--locked", "--offline", *manifest], cwd=ROOT, check=True,
                              capture_output=True, text=True).stdout

    # Every package any feature could bring, to look up those the tree names.
    meta = json.loads(run("metadata", "--format-version", "1", "--filter-platform", TARGET, "--all-features"))
    # What the crates' binaries link, with the features cargo unifies across every crate one build builds (`built`):
    # their normal dependencies on Linux on x86-64, procedural macros (which only run at build time) aside. cargo tree
    # prints one tree per crate, each root at depth 0.
    linked, root, below = set(), None, None
    selected = [a for crate in sorted(set(built) | set(crates)) for a in ("-p", crate)]
    tree = run("tree", *selected, *(["--features", features] if features else []), "-e", "normal", "--target", TARGET,
               "--prefix", "depth", "-f", "{p}", "--no-dedupe")
    for line in tree.splitlines():
        found = re.match(r"^(\d+)(\S+) v(\S+)", line)
        if not found:
            continue
        depth, name, version = int(found.group(1)), found.group(2), found.group(3)
        # Below a procedural macro, everything is its own build-time dependency (reached at run time, it shows again
        # on a path of its own).
        if below is not None and depth > below:
            continue
        below = None
        if depth == 0:
            root = name
        elif "(proc-macro)" in line:
            below = depth
        elif root in crates:
            linked.add((name, version))
    members = set(meta["workspace_members"])
    workspace = Path(meta["workspace_root"])
    packages = []
    for p in meta["packages"]:
        if p["id"] in members or (p["name"], p["version"]) not in linked:
            continue
        directory = str(Path(p["manifest_path"]).parent)
        extra = [os.path.relpath(Path(directory) / p["license_file"], directory)] if p.get("license_file") else []
        # A path dependency is the workspace's own, under its license, unless vendored (third_party/).
        vendored = os.path.relpath(directory, workspace).startswith("third_party/")
        if p["source"] is None and not vendored and not license_bearing(directory, license_files(directory) + extra):
            continue
        if p["source"] is None:
            where = f"{os.path.relpath(directory, workspace)}/ of the workspace this file's SOURCE names"
        elif p["source"].startswith("registry+https://github.com/rust-lang/crates.io-index"):
            where = f"https://crates.io/api/v1/crates/{p['name']}/{p['version']}/download"
        else:
            where = p["source"]
        packages.append((p["name"], p["version"], p.get("license") or "", directory, extra, where))
    return write(out, f"Rust crates linked into {', '.join(crates)}", packages)


def npm(out, directory):
    lock = json.loads((Path(directory) / "package-lock.json").read_text())
    packages = []
    for key, p in lock.get("packages", {}).items():
        if key and not p.get("dev"):
            packages.append((key.split("node_modules/")[-1], p.get("version", ""), p.get("license", ""),
                             str(Path(directory) / key), [], p.get("resolved", "the npm registry")))
    return write(out, f"npm packages bundled from {Path(directory).name}", packages)


def source(out, revision):
    """OUT/LICENSE, the repository's, and OUT/SOURCE, where the source of the image built from `revision` is."""
    out = Path(out)
    out.mkdir(parents=True, exist_ok=True)
    (out / "LICENSE").write_bytes((ROOT / "LICENSE").read_bytes())
    if re.fullmatch(r"[0-9a-f]{40}", revision):
        where = (f"It was built from commit {revision} of {REPOSITORY}, which its label org.opencontainers.image.revision "
                 f"names too. That commit is the complete corresponding source of easi.run's own programs in it:\n\n"
                 f"    git clone {REPOSITORY} && git -C easi.run checkout {revision}\n\n"
                 f"or {REPOSITORY}/tree/{revision} in a browser.")
    else:
        where = "It was built from a working tree, not from a published commit: its source is that tree."
    (out / "SOURCE").write_text(
        "easi.run is free software under the GNU Affero General Public License, version 3 only: LICENSE, beside this "
        "file.\n\n" + where + "\n\nThe other files here hold the license and notice texts of what else the image "
        "ships: the Rust crates its programs link, and, where it has them, the client's npm packages, the display's "
        "encoders and the agent's Cua Driver. Those of its Debian packages are under /usr/share/doc/<package>/.\n")


def cua_directory():
    """The pinned Cua Driver release's texts here (the root Dockerfile pins the release)."""
    version = re.search(r"cua-driver-rs-v([0-9][^/\s]*)/", (ROOT / "Dockerfile").read_text()).group(1)
    return FALLBACK / f"cua-driver-rs-{version}"


def image(reference, role, revision=""):
    """What is wrong with `reference`'s notices: each file its role needs is there and not empty, its LICENSE and Cua
    Driver texts are the repository's, its reports are whole (report), and its SOURCE names the commit its revision
    label names, and `revision` when given."""
    label = subprocess.run(["docker", "image", "inspect", "--format",
                            '{{index .Config.Labels "org.opencontainers.image.revision"}}', reference],
                           check=True, capture_output=True, text=True).stdout.strip()
    with tempfile.TemporaryDirectory() as temp:
        container = subprocess.run(["docker", "create", reference], check=True, capture_output=True,
                                   text=True).stdout.strip()
        try:
            archive = Path(temp) / "doc.tar"
            with archive.open("wb") as sink:
                subprocess.run(["docker", "cp", f"{container}:/{DOC}", "-"], check=True, stdout=sink)
        finally:
            subprocess.run(["docker", "rm", container], check=True, capture_output=True)
        with tarfile.open(archive) as tar:
            members = {m.name.split("/", 1)[1]: m for m in tar.getmembers() if "/" in m.name and m.isfile()}

            def read(name):
                return tar.extractfile(members[name]).read()

            problems = [f"/{DOC}/{f} is missing or empty" for f in ["LICENSE", "SOURCE", *REQUIRED[role]]
                        if f not in members or members[f].size == 0]
            if problems:
                return [f"{reference}: {p}" for p in problems]
            if read("LICENSE") != (ROOT / "LICENSE").read_bytes():
                problems.append(f"/{DOC}/LICENSE is not the repository's LICENSE")
            for name in REQUIRED[role]:
                if name.startswith("cua-driver/") and read(name) != (cua_directory() / name.split("/", 1)[1]).read_bytes():
                    problems.append(f"/{DOC}/{name} is not {cua_directory().relative_to(ROOT)}'s")
                elif name.endswith(("rust-crates.md", "npm-packages.md")):
                    problems += [f"/{DOC}/{name}: {p}" for p in report(read(name).decode())]
            for commit in {c for c in (label, revision) if c}:
                if f"commit {commit} " not in read("SOURCE").decode():
                    problems.append(f"/{DOC}/SOURCE does not name commit {commit}")
    return [f"{reference}: {p}" for p in problems]


def main():
    mode, args = (sys.argv[1], sys.argv[2:]) if len(sys.argv) > 2 else ("", [])
    options = {"--built-with": "", "--features": ""}
    while mode == "cargo" and len(args) >= 2 and args[-2] in options:
        options[args[-2]] = args[-1]
        args = args[:-2]
    if mode == "cargo" and len(args) >= 2:
        missing = cargo(args[0], args[1:], options["--built-with"].split(",") if options["--built-with"] else [],
                        options["--features"])
    elif mode == "npm" and len(args) == 2:
        missing = npm(*args)
    elif mode == "source" and len(args) in (1, 2):
        source(args[0], args[1] if len(args) == 2 else "")
        missing = []
    elif mode == "image" and len(args) in (2, 3) and args[1] in REQUIRED:
        missing = image(*args)
    else:
        sys.exit(__doc__)
    if missing:
        sys.exit(("" if mode == "image" else "no license text:\n  ") + "\n  ".join(missing))


if __name__ == "__main__":
    main()
