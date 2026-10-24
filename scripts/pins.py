#!/usr/bin/env python3
"""Checks that the pins CONTRIBUTING.md asks to move together do:

- every base image a Dockerfile builds from is pinned by digest, with one tag and digest for each of its flavours
  (`node:26` and `node:26-slim` are two, which must name one version) across all the Dockerfiles and the other files that pin an image by digest
  (PINNED: the bench and demo Compose files, the security tests, the benchmark runner and its docs, the site's
  release script);
- the benchmark runner's `playwright` npm package (package.json and its lock) is the version of its Playwright image;
- the `rust` image's version is the toolchain's channel in rust-toolchain.toml, and Cargo.toml's `rust-version` is
  its major and minor version;
- the benchmark baseline (bench/kasmvnc/Dockerfile) installs the same Google Chrome package as the browser image
  (browser/Dockerfile): the same URL by the same SHA-256. Otherwise a Chrome bump in one image alone would have the
  benchmark compare two Chrome versions while claiming one."""

import json
import re
import subprocess
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CHROME = re.compile(r"^ADD --checksum=(sha256:[0-9a-f]{64}) \\\n\s+(https://dl\.google\.com/\S+\.deb) \\$", re.M)
FROM = re.compile(r"^FROM\s+(?:--platform=\S+\s+)?(\S+)(?:\s+AS\s+(\S+))?\s*$", re.M | re.I)
BASE = re.compile(r"([^\s:@]+(?::\d+/[^\s:@]+)?)(?::([^\s@]+))?@(sha256:[0-9a-f]{64})")
RUST = "docker.io/library/rust"
# The files outside the Dockerfiles that pin an image by digest: each `image@sha256:` reference in them is checked
# with the Dockerfiles' pins.
PINNED = ("compose.bench.yaml", "demo/compose.yaml", "tests/tests/security.rs", "bench/runner/run.py",
          "docs/benchmark.md", "site/deploy/release.sh")
# The benchmark runner's npm manifest and lock, whose `playwright` version is its Playwright image's.
PLAYWRIGHT_NPM = ("bench/runner/package.json", "bench/runner/package-lock.json")
PLAYWRIGHT = "mcr.microsoft.com/playwright"
# An image reference inside such a file, which may sit in quotes: the whole digest, and nothing glued to it.
REFERENCE = re.compile(r"([\w./-]+(?::\d+/[\w./-]+)?)(?::([\w.-]+))?@(sha256:[0-9a-f]{64})(?![\w.:@/-])")


def canonical(image):
    """`image` as Docker names it in full, so `debian` and `docker.io/library/debian` are one image."""
    first, _, rest = image.partition("/")
    if not rest or ("." not in first and ":" not in first and first != "localhost"):
        image = f"docker.io/{image}"
    registry, _, path = image.partition("/")
    return f"{registry}/library/{path}" if registry == "docker.io" and "/" not in path else image


def flavour(tag):
    """The tag without its version numbers: `26-slim` is `-slim`, `26` and `` are ``, `v1.61.1-noble` is `v-noble`."""
    return re.sub(r"[0-9][0-9.]*", "", tag)


def playwright_npm():
    """The `playwright` versions the runner's package.json and lock name."""
    manifest = json.loads((ROOT / PLAYWRIGHT_NPM[0]).read_text())
    lock = json.loads((ROOT / PLAYWRIGHT_NPM[1]).read_text())
    return {manifest["devDependencies"]["playwright"], lock["packages"]["node_modules/playwright"]["version"]}


def chrome(path):
    found = CHROME.findall((ROOT / path).read_text())
    if len(found) != 1:
        sys.exit(f"{path}: expected one Google Chrome ADD --checksum line, found {len(found)}")
    return found[0]


def dockerfiles():
    # Untracked ones too (not ignored), so a new Dockerfile is checked before it is added.
    out = subprocess.run(["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard", "*Dockerfile"],
                         cwd=ROOT, capture_output=True, check=True)
    return sorted(p for p in out.stdout.decode().split("\0") if p and Path(p).name == "Dockerfile")


def main():
    errors = []
    pins = {}
    for path in dockerfiles():
        stages = set()
        for reference, stage in FROM.findall((ROOT / path).read_text()):
            # An earlier stage of the same file, or the empty image, is no base to pin.
            if reference.lower() != "scratch" and reference.lower() not in stages:
                pinned = BASE.fullmatch(reference)
                if pinned is None:
                    errors.append(f"{path}: FROM {reference} is not pinned by digest")
                else:
                    image, tag, digest = pinned.groups()
                    key = (canonical(image), flavour(tag or ""))
                    pins.setdefault(key, {}).setdefault((tag or "", digest), []).append(path)
            if stage:
                stages.add(stage.lower())
    for path in PINNED:
        text = (ROOT / path).read_text()
        found = REFERENCE.findall(text)
        if not found:
            errors.append(f"{path}: no image pinned by digest; update PINNED in scripts/pins.py")
        if len(found) != text.count("@sha256:"):
            errors.append(f"{path}: a digest reference is not a whole `image[:tag]@sha256:<64 hex>`")
        for image, tag, digest in found:
            pins.setdefault((canonical(image), flavour(tag)), {}).setdefault((tag, digest), []).append(path)
    versions = {}
    for (image, _), uses in pins.items():
        versions.setdefault(image, set()).update(tuple(re.findall(r"[0-9][0-9.]*", tag)) for tag, _ in uses)
    for image, found in sorted(versions.items()):
        if len(found) > 1:
            errors.append(f"{image}'s flavours name different versions: {', '.join('.'.join(v) or '(none)' for v in sorted(found))}")
    rust = {tag: paths for (image, _), uses in pins.items() if image == RUST for tag, paths in uses.items()}
    if not rust:
        errors.append(f"no Dockerfile builds from {RUST}")
    for (image, _), uses in sorted(pins.items()):
        if len(uses) > 1:
            listed = "; ".join(f"{tag or '(no tag)'}@{digest} in {', '.join(paths)}"
                               for (tag, digest), paths in sorted(uses.items()))
            errors.append(f"{image} is pinned more than one way: {listed}")
    channel = tomllib.loads((ROOT / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
    minimum = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["rust-version"]
    for tag, _ in rust:
        version = tag.split("-")[0]
        if version != channel:
            errors.append(f"{RUST}:{tag} is Rust {version}, but rust-toolchain.toml's channel is {channel}")
        if ".".join(version.split(".")[:2]) != minimum:
            errors.append(f"{RUST}:{tag} is Rust {version}, but Cargo.toml's rust-version is {minimum}")
    images = {tag.removeprefix("v").split("-")[0] for (image, _), uses in pins.items() if image == PLAYWRIGHT
              for tag, _ in uses}
    if len(images) != 1 or playwright_npm() != images:
        errors.append(f"the playwright npm package ({', '.join(sorted(playwright_npm()))} in {' and '.join(PLAYWRIGHT_NPM)}) "
                      f"is not the version of its image {PLAYWRIGHT} ({', '.join(sorted(images)) or 'none'})")
    browser, baseline = chrome("browser/Dockerfile"), chrome("bench/kasmvnc/Dockerfile")
    if browser != baseline:
        errors.append(f"bench/kasmvnc/Dockerfile installs {baseline[1]} ({baseline[0]}), but browser/Dockerfile "
                      f"installs {browser[1]} ({browser[0]}): pin the same package in both")
    if errors:
        sys.exit("\n".join(errors))
    print(f"{len(pins)} images pinned one way each across {len(dockerfiles())} Dockerfiles and {len(PINNED)} other "
          f"files; Rust {channel}; one Google Chrome package")


if __name__ == "__main__":
    main()
