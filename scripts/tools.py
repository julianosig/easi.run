#!/usr/bin/env python3
"""Writes admin/src/tools.json: the driver tools an agent token may call, as the agent image's driver defines them.

    scripts/tools.py                 # the tools the snapshot names now, from easi-run/agent:dev
    scripts/tools.py NAME...         # these tools instead
    scripts/tools.py --check         # write nothing; fail when the live definitions differ from the snapshot
    AGENT_IMAGE=... scripts/tools.py # another agent image

It runs the image's driver in a throwaway container with no network, asks it for `tools/list`, and keeps each named
tool's name, title, description, input schema and annotations. Name only desktop tools a session without DevTools
runs. The agents end-to-end test (`tests/tests/agents.rs`) runs the check.
"""

import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

SNAPSHOT = Path(__file__).resolve().parent.parent / "admin" / "src" / "tools.json"
FIELDS = ("name", "title", "description", "inputSchema", "annotations")


def listed(image):
    messages = [
        {"jsonrpc": "2.0", "id": 0, "method": "initialize",
         "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "tools.py", "version": "1"}}},
        {"jsonrpc": "2.0", "method": "notifications/initialized"},
        {"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {}},
    ]
    # A local image only: one that is absent fails at once, and Docker's reason is part of the failure.
    with tempfile.TemporaryFile("w+") as errors:
        driver = subprocess.Popen(
            ["docker", "run", "--rm", "-i", "--pull", "never", "--network", "none", "--read-only", "--tmpfs", "/tmp",
             "--cap-drop", "ALL", "--security-opt", "no-new-privileges:true", "--entrypoint", "/usr/local/bin/agent-mcp",
             image],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=errors, text=True)
        try:
            driver.stdin.write("".join(json.dumps(m) + "\n" for m in messages))
            driver.stdin.flush()
        except BrokenPipeError:
            pass  # it already ended; its standard error says why
        try:
            for line in driver.stdout:
                message = json.loads(line)
                if message.get("id") == 1:
                    return message["result"]["tools"]
        finally:
            driver.kill()
            driver.wait()
        errors.seek(0)
        why = errors.read().strip()
    sys.exit(f"the driver never listed its tools: {why}" if why else "the driver never listed its tools")


def main():
    args = sys.argv[1:]
    check = args == ["--check"]
    current = json.loads(SNAPSHOT.read_text())
    names = [] if check else args
    names = names or [t["name"] for t in current["tools"]]
    tools = {t["name"]: t for t in listed(os.environ.get("AGENT_IMAGE", "easi-run/agent:dev"))}
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
    # Which driver release wrote them is the agent image's pin (agent/Dockerfile), not a field here to go stale.
    SNAPSHOT.write_text(json.dumps({"tools": kept}, indent=2, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main()
