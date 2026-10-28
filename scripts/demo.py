#!/usr/bin/env python3
"""Checks that every list of the public demo's hosts names the same demos: ADMIN_DEMO_SITES in demo/compose.yaml
(each at https://NAME.play/), both host patterns in demo/nginx.conf, the certificate's names in demo/sites-cert.sh,
the pages in client/src/ui/demos.ts, the landing's tab names (site/messages/en/demo.json, `NAME.title` and
`NAME.line`), and the directories under demo/sites/ (besides `shared/`). A demo missing from one of them gets a
certificate error, is sent home, or shows no title (on the landing, an English one)."""

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DEMO = ROOT / "demo"


def one(pattern, text, where):
    found = re.findall(pattern, text, re.M)
    if len(found) != 1:
        sys.exit(f"{where}: expected one match of {pattern!r}, found {len(found)}")
    return found[0]


def lists():
    """Each list of demo names, by where it is."""
    out = {}
    compose = (DEMO / "compose.yaml").read_text()
    sites = one(r"^\s*ADMIN_DEMO_SITES:\s*(\S+)\s*$", compose, "demo/compose.yaml")
    pairs = [entry.split("=", 1) for entry in sites.split(",")]
    wrong = [f"{name}={url}" for name, url in pairs if url != f"https://{name}.play/"]
    if wrong:
        sys.exit(f"demo/compose.yaml: ADMIN_DEMO_SITES entries not at https://NAME.play/: {wrong}")
    out["demo/compose.yaml ADMIN_DEMO_SITES"] = [name for name, _ in pairs]
    nginx = (DEMO / "nginx.conf").read_text()
    patterns = re.findall(r"\(\?<demo>([a-z|-]+)\)", nginx)
    if len(patterns) != 2:
        sys.exit(f"demo/nginx.conf: expected two (?<demo>...) host patterns, found {len(patterns)}")
    for i, pattern in enumerate(patterns, 1):
        out[f"demo/nginx.conf pattern {i}"] = pattern.split("|")
    out["demo/sites-cert.sh hosts"] = one(r'^hosts="([^"]*)"$', (DEMO / "sites-cert.sh").read_text(),
                                          "demo/sites-cert.sh").split()
    demos = (ROOT / "client" / "src" / "ui" / "demos.ts").read_text()
    block = re.search(r"^(?:export )?const DEMOS\b[^\n]*\{\n(.*?)^\}\)*;", demos, re.M | re.S)
    if block is None:
        sys.exit("client/src/ui/demos.ts: no DEMOS table")
    out["client/src/ui/demos.ts DEMOS"] = re.findall(r"^  '?([a-z-]+)'?: \{", block.group(1), re.M)
    tabs = json.loads((ROOT / "site" / "messages" / "en" / "demo.json").read_text())
    titled = sorted({key.split(".")[0] for key in tabs if key.endswith((".title", ".line"))})
    lined = [name for name in titled if f"{name}.title" in tabs and f"{name}.line" in tabs]
    out["site/messages/en/demo.json tabs"] = lined
    out["demo/sites/ directories"] = [d.name for d in (DEMO / "sites").iterdir() if d.is_dir() and d.name != "shared"]
    return out


def main():
    found = lists()
    every = set().union(*found.values())
    failures = []
    for where, names in found.items():
        if len(names) != len(set(names)):
            failures.append(f"{where}: a demo listed twice: {names}")
        if missing := sorted(every - set(names)):
            failures.append(f"{where}: missing {', '.join(missing)}")
    if failures:
        print("the demo's host lists differ:", *failures, sep="\n  ")
        sys.exit(1)
    print(f"the demo's {len(found)} host lists name the same {len(every)} demos")


if __name__ == "__main__":
    main()
