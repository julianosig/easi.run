#!/usr/bin/env python3
"""Checks that the benchmark baseline (bench/kasmvnc/Dockerfile) installs the same Google Chrome package as the browser
image (browser/Dockerfile): the same URL by the same SHA-256. Otherwise a Chrome bump in one image alone would have the
benchmark compare two Chrome versions while claiming one."""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PIN = re.compile(r"^ADD --checksum=(sha256:[0-9a-f]{64}) \\\n\s+(https://dl\.google\.com/\S+\.deb) \\$", re.M)


def pin(path):
    found = PIN.findall((ROOT / path).read_text())
    if len(found) != 1:
        sys.exit(f"{path}: expected one Google Chrome ADD --checksum line, found {len(found)}")
    return found[0]


def main():
    browser, baseline = pin("browser/Dockerfile"), pin("bench/kasmvnc/Dockerfile")
    if browser != baseline:
        sys.exit(f"bench/kasmvnc/Dockerfile installs {baseline[1]} ({baseline[0]}), but browser/Dockerfile installs "
                 f"{browser[1]} ({browser[0]}): pin the same package in both")


if __name__ == "__main__":
    main()
