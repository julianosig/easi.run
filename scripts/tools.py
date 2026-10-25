#!/usr/bin/env python3
"""Writes admin/src/tools.json: the driver tools an agent token may call, as the agent image's driver defines them.

    scripts/tools.py                 # the tools the snapshot names now, from easi-run-bench/agent:dev
    scripts/tools.py NAME...         # these tools instead
    scripts/tools.py --check         # write nothing; fail when the live definitions differ from the snapshot
    scripts/tools.py --site          # write nothing; fail when the docs' list differs from the snapshot (no Docker)
    AGENT_IMAGE=... scripts/tools.py # another agent image

It runs the image's driver in a throwaway container with no network, asks it for `tools/list`, and keeps each named
tool's name, title, description, input schema and annotations, and writes their names for the documentation pages
(site/src/docs/tools.ts). Name only desktop tools a session without DevTools runs. The agents end-to-end test
(`tests/tests/agents.rs`) runs the check, and scripts/check.sh the docs' one.
"""

import json
import os
import re
import subprocess
import sys
import tempfile
import threading
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SNAPSHOT = ROOT / "admin" / "src" / "tools.json"
SITE = ROOT / "site" / "src" / "docs" / "tools.ts"
FIELDS = ("name", "title", "description", "inputSchema", "annotations")
# Seconds the driver has to list its tools (it takes about one): past them it is stopped, so a driver that never
# answers fails the check instead of hanging it.
DEADLINE = 60


def listed(image):
    messages = [
        {"jsonrpc": "2.0", "id": 0, "method": "initialize",
         "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "tools.py", "version": "1"}}},
        {"jsonrpc": "2.0", "method": "notifications/initialized"},
        {"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {}},
    ]
    # A local image only: one that is absent fails at once, and Docker's reason is part of the failure. The container
    # is named so that it is removed with the client, however the client ends.
    name = f"easi-run-tools-{os.getpid()}"
    expired = threading.Event()
    with tempfile.TemporaryFile("w+") as errors:
        driver = subprocess.Popen(
            ["docker", "run", "--rm", "-i", "--name", name, "--pull", "never", "--network", "none", "--read-only",
             "--tmpfs", "/tmp", "--cap-drop", "ALL", "--security-opt", "no-new-privileges:true",
             "--entrypoint", "/usr/local/bin/agent-mcp", image],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=errors, text=True)

        def expire():
            expired.set()
            driver.kill()

        timer = threading.Timer(DEADLINE, expire)
        timer.start()
        try:
            driver.stdin.write("".join(json.dumps(m) + "\n" for m in messages))
            driver.stdin.flush()
        except BrokenPipeError:
            pass  # it already ended; its standard error says why
        try:
            for line in driver.stdout:
                try:
                    message = json.loads(line)
                except ValueError:
                    sys.exit(f"the driver wrote a line that is not JSON: {line.strip()[:200]!r}")
                if message.get("id") == 1:
                    return message["result"]["tools"]
        finally:
            timer.cancel()
            driver.kill()
            driver.wait()
            subprocess.run(["docker", "rm", "-f", name], capture_output=True)
        errors.seek(0)
        why = errors.read().strip()
    never = "the driver never listed its tools"
    if expired.is_set():
        never = f"the driver did not list its tools within {DEADLINE} s"
    sys.exit(f"{never}: {why}" if why else never)


def site(names):
    return ("// The desktop tools a session without DevTools runs, as admin/src/tools.json pins them. Written by\n"
            "// scripts/tools.py; scripts/check.sh fails when the two differ.\n"
            f"export const TOOLS: readonly string[] = [{', '.join(repr(n) for n in names)}];\n")


def main():
    args = sys.argv[1:]
    check = args == ["--check"]
    current = json.loads(SNAPSHOT.read_text())
    if args == ["--site"]:
        pinned = [t["name"] for t in current["tools"]]
        listed_there = re.findall(r"'([a-z_]+)'", SITE.read_text())
        if listed_there != pinned:
            sys.exit(f"{SITE.relative_to(ROOT)} lists {listed_there}, but {SNAPSHOT.name} pins {pinned}: "
                     "run scripts/tools.py, or write the pinned names there")
        return
    names = [] if check else args
    names = names or [t["name"] for t in current["tools"]]
    tools = {t["name"]: t for t in listed(os.environ.get("AGENT_IMAGE", "easi-run-bench/agent:dev"))}
    missing = [n for n in names if n not in tools]
    if missing:
        sys.exit(f"the driver lists no {', '.join(missing)}")
    kept = [{f: tools[n][f] for f in FIELDS if f in tools[n]} for n in names]
    if check:
        changed = [new["name"] for old, new in zip(current["tools"], kept) if old != new]
        if changed:
            sys.exit(f"the live driver defines {', '.join(changed)} differently from {SNAPSHOT.name}; "
                     "review them, then run scripts/tools.py")
        return
    # Which driver release wrote them is the agent image's pin (the root Dockerfile), not a field here to go stale.
    SNAPSHOT.write_text(json.dumps({"tools": kept}, indent=2, ensure_ascii=False) + "\n")
    SITE.write_text(site(names))


if __name__ == "__main__":
    main()
