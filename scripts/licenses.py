#!/usr/bin/env python3
"""Writes docs/licenses.md from the image SBOMs (docs/sbom/*.spdx.json) and the Rust dependency graph.

Runtime inventory: for each runtime image, every Debian package with the license terms its copyright file names,
and for each Rust binary, every crate it links (normal dependencies only). Bench inventory: the baseline and fixture
images. Landing site inventory: every npm package of the static landing page. Development inventory: build-only
crates and the client's npm dev dependencies.

A Debian copyright file lists every license of every file in a package (packaging scripts, documentation, tests), so
a GPL term there does not mean the shipped library is GPL. Each runtime package with a GPL term is therefore also
classified by hand in CLASSIFIED below; anything unclassified is reported as such and fails the check, and so does a
classification of a package no runtime image ships with a GPL term (it would pass that package unreviewed should it
return with one).
"""

import json
import re
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SBOM = ROOT / "docs" / "sbom"
RUNTIME_IMAGES = ["display", "browser", "agent", "proxy", "admin", "controller"]
BENCH_IMAGES = ["baseline", "fixtures"]
RUST_BINARIES = {
    "display": ["display"],
    "admin": ["admin"],
    "controller": ["controller"],
    "proxy": ["proxy"],
    "browser": ["browser-launch"],
    "agent": ["agent"],
}
PERMISSIVE = {"MIT", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "ISC", "Zlib", "0BSD", "Unicode-3.0", "Unlicense",
              "BSL-1.0", "CC0-1.0", "MIT-0", "BSD-3-Clause-Clear", "CDLA-Permissive-2.0"}

# The display's encoders (two video, one sound), with the license of what the image ships. All are compiled into the
# display binary from source, so the SBOM's package list cannot show them. Their versions come from where the build
# takes them: the source the openh264-sys2 crate bundles, and VERSION in the two build scripts.
def encoders():
    return {
        "openh264-sys2": (f"OpenH264 {openh264_version()}, BSD-2-Clause, compiled from source into the display "
                          "binary only in builds with the h264 feature, which the published images leave off (Cisco's "
                          "H.264 patent coverage applies only to its own prebuilt binary; see below)"),
        "svt-av1": (f"SVT-AV1 {script_version('svt-av1.sh')}, BSD-3-Clause-Clear with the AOMedia patent license (its "
                    "bundled libaom code is BSD-2-Clause with the same patent license, fastfeat BSD-3-Clause, "
                    "safestringlib MIT); compiled from its checksummed release source archive (scripts/svt-av1.sh) "
                    "into a static library linked into the display binary, so no package in the SBOM carries it"),
        "opus": (f"libopus {script_version('opus.sh')}, BSD-3-Clause (with the patent licenses its authors grant for "
                 "Opus, RFC 6716); compiled from its checksummed release source archive (scripts/opus.sh) into a "
                 "static library linked into the display binary, so no package in the SBOM carries it"),
    }


# The agent's prebuilt Cua Driver: the one binary the agent image extracts from the release archive it pins by
# checksum. The SBOM's package list cannot show it either; its version comes from that archive's URL.
def cua_driver():
    found = re.search(r"/releases/download/cua-driver-rs-v([0-9][^/\s]*)/", (ROOT / "Dockerfile").read_text())
    if not found:
        sys.exit("Dockerfile pins no cua-driver-rs release")
    return (f"Cua Driver (`cua-driver-rs`) {found.group(1)}, MIT (Cua AI, Inc.; its repository's LICENSE.md and its "
            "Cargo workspace's license); the prebuilt `cua-driver` binary, extracted alone from its checksummed "
            "release archive (the root Dockerfile) into the agent image, so no package in the SBOM carries it")


def script_version(script):
    """The VERSION a build script under scripts/ builds."""
    found = re.search(r"^VERSION=(\S+)$", (ROOT / "scripts" / script).read_text(), re.MULTILINE)
    if not found:
        sys.exit(f"scripts/{script} sets no VERSION")
    return found.group(1)


def openh264_version():
    """The OpenH264 release the locked openh264-sys2 crate bundles, from its codec_ver.h (the crate resolves only with
    the display's h264 feature)."""
    crate = next(p for p in metadata("--features", "display/h264")["packages"] if p["name"] == "openh264-sys2")
    header = Path(crate["manifest_path"]).parent / "upstream" / "codec" / "api" / "wels" / "codec_ver.h"
    parts = [re.search(rf"^#define OPENH264_{part} \((\d+)\)$", header.read_text(), re.MULTILINE)
             for part in ("MAJOR", "MINOR", "REVISION")]
    if not all(parts):
        sys.exit(f"{header} names no OpenH264 version")
    return ".".join(m.group(1) for m in parts)


# Why a runtime package whose copyright file names a GPL term is acceptable: the license of what the image ships.
CLASSIFIED = {
    "base-files": "data files (os-release, legal notices); no program",
    "netbase": "data files (/etc/protocols, /etc/services); no program",
    "ca-certificates": ("certificate data, MPL-2.0; the GPL term covers the update-ca-certificates script, which the "
                        "distroless images leave out and the browser image keeps with no shell to run it"),
    "libc6": "glibc, LGPL-2.1+ (dynamically linked); GPL terms cover build tooling",
    "libc-bin": "glibc utilities (ldconfig, locale data), LGPL-2.1+",
    "libssl3t64": "OpenSSL 3, Apache-2.0; the GPL term covers old documentation",
    "openssl-provider-legacy": "OpenSSL 3, Apache-2.0",
    "libzstd1": "zstd, BSD-3-Clause OR GPL-2.0 (BSD chosen)",
    "liblz4-1": "lz4 library, BSD-2-Clause (the GPL covers the CLI, not shipped)",
    "liblzma5": "xz library, 0BSD/public domain (GPL covers scripts, not shipped)",
    "libbz2-1.0": "bzip2 library, BSD-like",
    "libxxhash0": "xxHash library, BSD-2-Clause (GPL covers the CLI, not shipped)",
    "libsqlite3-0": "SQLite, public domain",
    "libselinux1": "libselinux, public domain",
    "libaudit1": "libaudit, LGPL-2.1",
    "libaudit-common": "libaudit configuration data, LGPL-2.1",
    "libcap2": "libcap, BSD-3-Clause OR GPL-2.0 (BSD chosen)",
    "libcap-ng0": "libcap-ng, LGPL-2.1+",
    "libapparmor1": "libapparmor, LGPL-2.1",
    "libsystemd0": "libsystemd, LGPL-2.1+",
    "libudev1": "libudev, LGPL-2.1+",
    "libblkid1": "libblkid, LGPL-2.1+",
    "libmount1": "libmount, LGPL-2.1+",
    "libgmp10": "GMP, LGPL-3.0+ OR GPL-2.0+ (LGPL chosen)",
    "libnettle8t64": "Nettle, LGPL-3.0+ OR GPL-2.0+ (LGPL chosen)",
    "libhogweed6t64": "Nettle, LGPL-3.0+ OR GPL-2.0+ (LGPL chosen)",
    "libp11-kit0": "p11-kit, BSD-3-Clause",
    "libidn2-0": "libidn2, LGPL-3.0+ OR GPL-2.0+ (LGPL chosen)",
    "libunistring5": "libunistring, LGPL-3.0+ OR GPL-2.0+ (LGPL chosen)",
    "libelf1t64": "elfutils libelf, LGPL-3.0+ OR GPL-2.0+ (LGPL chosen)",
    "libdatrie1": "libdatrie, LGPL-2.1+",
    "libthai0": "libthai, LGPL-2.1+",
    "libthai-data": "libthai data, LGPL-2.1+",
    "at-spi2-core": "AT-SPI bus launcher and registry, LGPL-2.1+",
    "at-spi2-common": "AT-SPI data, LGPL-2.1+",
    "libatspi2.0-0t64": "AT-SPI library, LGPL-2.1+",
    "libatk1.0-0t64": "ATK, LGPL-2.1+",
    "libatk-bridge2.0-0t64": "ATK bridge, LGPL-2.1+",
    "dbus-bin": "D-Bus tools, AFL-2.1 OR GPL-2.0+ (AFL chosen)",
    "dbus-daemon": "D-Bus daemon, AFL-2.1 OR GPL-2.0+ (AFL chosen)",
    "dbus-session-bus-common": "D-Bus configuration, AFL-2.1 OR GPL-2.0+",
    "libdbus-1-3": "libdbus, AFL-2.1 OR GPL-2.0+ (AFL chosen)",
    "dconf-gsettings-backend": "dconf GSettings module, LGPL-2.1+ (the GPL-3 service is removed)",
    "fonts-dejavu-core": "font, Bitstream Vera license (the GPL term covers packaging scripts)",
    "fonts-dejavu-mono": "font, Bitstream Vera license",
    "fonts-liberation": "font, SIL OFL-1.1",
    "fonts-liberation2": "font, SIL OFL-1.1",
    "fonts-noto-core": "font, SIL OFL-1.1",
    "fonts-noto-cjk": "font, SIL OFL-1.1",
    "fonts-noto-mono": "font, SIL OFL-1.1",
    "hicolor-icon-theme": "icon theme directory skeleton (data)",
    "libcolord2": "colord library, LGPL-2.1+",
    "libcups2t64": "CUPS library, Apache-2.0 with GPL2/LGPL2 exception",
    "libcom-err2": "e2fsprogs com_err, MIT-style",
    "libkeyutils1": "keyutils library, LGPL-2.1+",
    "libglib2.0-0t64": "GLib, LGPL-2.1+",
    "libgtk-3-0t64": "GTK 3, LGPL-2.1+",
    "libgtk-3-common": "GTK 3 data, LGPL-2.1+",
    "libharfbuzz0b": "HarfBuzz, MIT",
    "libgdk-pixbuf-2.0-0": "gdk-pixbuf, LGPL-2.1+",
    "libgdk-pixbuf2.0-common": "gdk-pixbuf data, LGPL-2.1+",
    "libpulse0": "PulseAudio client library, LGPL-2.1+",
    "pulseaudio": ("PulseAudio daemon, LGPL-2.1+ as built: the image keeps three modules (the native protocol, the "
                   "pipe sink, suspend on idle), none of the GPL-linked ones, and no tools"),
    "libltdl7": "libltdl, LGPL-2.1+ (GPL covers libtool itself, not shipped)",
    "libtdb1": "TDB, LGPL-3.0+",
    "libsndfile1": "libsndfile, LGPL-2.1+",
    "libflac14": "FLAC library, BSD-3-Clause",
    "libmp3lame0": "LAME, LGPL-2.0+",
    "libgbm1": "Mesa GBM, MIT",
    "libfreetype6": "FreeType, FTL OR GPL-2.0 (FTL chosen)",
    "libpng16-16t64": "libpng, libpng license",
    "liblcms2-2": "Little CMS, MIT",
    "libffi8": "libffi, MIT",
    "libglvnd0": "libglvnd, MIT-style",
    "mesa-libgallium": "Mesa, MIT",
    "libegl1": "libglvnd EGL dispatch, MIT-style (the GPL terms cover build tooling), for WebGL on Mesa",
    "libgles2": "libglvnd GLES dispatch, MIT-style (the GPL terms cover build tooling), for WebGL on Mesa",
    "libegl-mesa0": "Mesa EGL, MIT",
    "libgraphite2-3": "Graphite, LGPL-2.1+ OR MPL-1.1 OR GPL-2.0+ (LGPL chosen)",
    "libz3-4": "Z3, MIT (the GPL term covers packaging)",
    "libsensors5": "lm-sensors library, LGPL-2.1+",
}

# Proprietary packages the owner approved, and the only images that may carry them.
PROPRIETARY = {"google-chrome-stable": {"browser"}}

RESIDUAL_NOTE = {
    "libgcc-s1, libstdc++6": "GPL-3.0 with the GCC Runtime Library Exception, which lets any program link them.",
    "google-chrome-stable": "Google Chrome is proprietary (Google Chrome Terms of Service). It is the owner-approved "
                            "exception to the permissive-license rule, for the browser image only, chosen so the "
                            "browser is an ordinary Google Chrome to the sites it visits. The image is built from "
                            "Google's signed package by checksum; it is not redistributed in this repository.",
}


def lic(package):
    for key in ("licenseConcluded", "licenseDeclared"):
        value = package.get(key)
        if value and value != "NOASSERTION":
            return value
    return "NOASSERTION"


def packages(image):
    data = json.loads((SBOM / f"{image}.spdx.json").read_text())
    out = []
    for p in data.get("packages", []):
        if p.get("SPDXID") == "SPDXRef-DocumentRoot" or p["name"].startswith("/"):
            continue
        out.append((p["name"], p.get("versionInfo", ""), lic(p)))
    return sorted(set(out))


def gpl_term(license_text):
    return re.search(r"(^|[^L])A?GPL", license_text) is not None


def metadata(*args):
    return json.loads(subprocess.run(["cargo", "metadata", "--format-version", "1", "--locked", *args], cwd=ROOT,
                                     check=True, capture_output=True, text=True).stdout)


def rust_crates(meta):
    by_id = {p["id"]: p for p in meta["packages"]}
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    # The project's own crates; everything else is third-party, vendored copies (third_party/) included.
    members = set(meta["workspace_members"])
    names = {p["name"]: p["id"] for p in meta["packages"] if p["id"] in members}

    def closure(root, kinds):
        seen, stack = set(), [root]
        while stack:
            node = stack.pop()
            if node in seen:
                continue
            seen.add(node)
            for dep in nodes[node]["deps"]:
                if any(k["kind"] in kinds for k in dep["dep_kinds"]):
                    stack.append(dep["pkg"])
        return seen

    runtime, everything = {}, set()
    for image, crates in RUST_BINARIES.items():
        ids = set()
        for crate in crates:
            ids |= closure(names[crate], {None})
        runtime[image] = sorted((by_id[i]["name"], by_id[i]["version"], by_id[i].get("license") or "?") for i in ids
                                if i not in members)
    for crate_id in names.values():
        everything |= closure(crate_id, {None, "build", "dev"})
    build_only = sorted({(by_id[i]["name"], by_id[i]["version"], by_id[i].get("license") or "?") for i in everything
                         if i not in members} - {c for v in runtime.values() for c in v})
    return runtime, build_only


def permissive(expression):
    terms = re.split(r"\s+(?:OR|AND)\s+|/|[()]", expression)
    terms = [t.strip() for t in terms if t.strip()]
    if " OR " in expression or "/" in expression:
        return any(t in PERMISSIVE or t == "MPL-2.0" for t in terms)
    return all(t in PERMISSIVE or t == "MPL-2.0" for t in terms)


def npm_packages(directory):
    """Every package in `directory`'s npm lock: name, version, license, and whether it is a development one."""
    lock = json.loads((ROOT / directory / "package-lock.json").read_text())
    return sorted((k.split("node_modules/")[-1], v.get("version", ""), v.get("license", "?"), bool(v.get("dev")))
                  for k, v in lock.get("packages", {}).items() if k)


def check_bundled(where, bundled, failures):
    """The listing of npm packages served with a page, failing any that is neither permissive nor a typeface."""
    lines = []
    for name, version, license_text, _ in bundled:
        # A typeface under the SIL Open Font License may be bundled and served with the page; the license is for fonts.
        font = name.startswith("@fontsource") and license_text == "OFL-1.1"
        if not permissive(license_text) and not font:
            failures.append(f"{where}: {name} {version} ({license_text})")
        lines.append(f"- {name} {version}: {license_text}")
    return lines


def main():
    lines = ["# Licenses", "",
             "Generated by `scripts/licenses.py` from `docs/sbom/*.spdx.json` (syft) and the Cargo lock graph. Do not "
             "edit by hand.", ""]
    failures = []
    runtime_crates, build_only = rust_crates(metadata())
    lines += ["## Runtime inventory", "",
              "Everything in the runtime images: the admin, the controller, and the images each session runs. "
              "The easi.run code and its client are AGPL-3.0-only; everything they use keeps its own license, listed "
              "below (the encoders, the agent's driver, the client's packages, each image's packages, and the Rust "
              "crates each binary links). "
              "A Debian copyright file names every license of every file in a package, so each package with a GPL "
              "term is listed with what the image actually ships.", "", "### Encoders", ""]
    for name, what in encoders().items():
        lines.append(f"- `{name}`: {what}.")
    lines += ["", "### Agent driver", "", f"- `cua-driver`: {cua_driver()}."]
    client = npm_packages("client")
    bundled = [p for p in client if not p[3]]
    lines += ["", "### Client", "",
              "The client's production npm packages, from which its build bundles what the page the admin image "
              f"serves uses ({len(bundled)}):", ""]
    lines += check_bundled("client", bundled, failures)
    lines.append("")
    classified = set()
    for image in RUNTIME_IMAGES:
        pkgs = packages(image)
        for name, _, _ in pkgs:
            if name in PROPRIETARY and image not in PROPRIETARY[name]:
                failures.append(f"{image}: {name} is proprietary, approved only for {sorted(PROPRIETARY[name])}")
        lines += [f"### `{image}`", "", f"{len(pkgs)} packages in the SBOM.", ""]
        flagged = [(n, v, terms) for n, v, terms in pkgs if gpl_term(terms)]
        classified |= {name for name, _, _ in flagged}
        if flagged:
            lines += ["| Package | Version | What ships |", "|---|---|---|"]
            for name, version, license_text in flagged:
                why = CLASSIFIED.get(name)
                if not why:
                    failures.append(f"{image}: {name} ({license_text[:80]})")
                    why = f"UNCLASSIFIED: {license_text[:120]}"
                lines.append(f"| {name} | {version} | {why} |")
            lines.append("")
        crates = runtime_crates.get(image, [])
        if crates:
            bad = [c for c in crates if not permissive(c[2])]
            lines += [f"Rust crates linked: {len(crates)}; all permissive: {'yes' if not bad else 'NO'}.", ""]
            for name, version, license_text in bad:
                failures.append(f"{image}: crate {name} {version} ({license_text})")
                lines.append(f"- {name} {version}: {license_text}")
            if bad:
                lines.append("")
    for name in sorted(set(CLASSIFIED) - classified):
        failures.append(f"CLASSIFIED: {name} has a GPL term in no runtime image")
    lines += ["### Residuals", ""]
    for name, note in RESIDUAL_NOTE.items():
        lines.append(f"- `{name}`: {note}")
    site = npm_packages("site")
    lines += ["", "## Landing site inventory", "",
              "The landing page's npm packages, all checked as bundled: its lock marks every one a development "
              f"dependency, and the build decides which reach the page that is served ({len(site)}):", ""]
    lines += check_bundled("site", site, failures)
    lines += ["", "## Bench inventory", "",
              "The `bench` profile only, never a runtime image: the KasmVNC baseline (GPL-2.0; built locally for "
              "measurement and not distributed, so no source offer is due; a distribution would follow GPL-2.0 "
              "section 3 with the KasmVNC and X.Org sources it builds on) and the fixture host.", ""]
    for image in BENCH_IMAGES:
        pkgs = packages(image)
        gpl = [n for n, _, terms in pkgs if gpl_term(terms)]
        lines.append(f"- `{image}`: {len(pkgs)} packages, {len(gpl)} with a GPL term (SBOM in `docs/sbom/{image}.spdx.json`).")
    lines += ["", "## Development inventory", "",
              f"Crates used only to build or test ({len(build_only)}), any license:", ""]
    by_license = defaultdict(list)
    for name, version, license_text in build_only:
        by_license[license_text].append(f"{name} {version}")
    for license_text in sorted(by_license):
        lines.append(f"- {license_text}: {', '.join(sorted(by_license[license_text]))}")
    dev = [p for p in client if p[3]]
    lines += ["", f"Client build tools (npm, development only, {len(dev)}):", ""]
    lines += [f"- {n} {v}: {terms}" for n, v, terms, _ in dev]
    lines += ["", "## Codec patents", "",
              "Copyright licenses do not settle patent rights. AV1 is covered by the Alliance for Open Media's "
              "royalty-free patent license, and the published display image encodes video only in AV1. H.264 is "
              "patent-encumbered: a build with the h264 feature compiles OpenH264 from source, and Cisco's patent "
              "coverage applies only to Cisco's prebuilt binary, not to a source build, so such a build needs its own "
              "patent position.", ""]
    (ROOT / "docs" / "licenses.md").write_text("\n".join(lines) + "\n")
    if failures:
        print("unclassified or non-permissive:", *failures, sep="\n  ")
        sys.exit(1)
    print("docs/licenses.md written; every runtime component is classified")


if __name__ == "__main__":
    main()
