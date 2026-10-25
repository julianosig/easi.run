#!/usr/bin/env python3
"""Checks that the docs' Configuration page (site/src/docs/config.ts) lists every setting the code reads, and nothing
else: each `ADMIN_*`, `CONTROLLER_*`, `IMAGE_*` and `STREAMS_*` name quoted in the admin's and the controller's
sources, and each variable compose.yaml reads from the shell or `.env`. Where the code states a
setting's default as a literal (`env("NAME", "default")`, `env("NAME").unwrap_or_else(|| "default".into())`, or
`${NAME:-default}` in a Compose file), the page's default is that one."""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SOURCES = ("admin/src", "controller/src")
COMPOSE = ("compose.yaml",)
TABLES = "site/src/docs/config.ts"
# Read by a library rather than by name in these sources: tracing's filter.
LIBRARY = {"RUST_LOG"}
NAME = re.compile(r'"((?:ADMIN|CONTROLLER|IMAGE|STREAMS)_[A-Z0-9_]+)"')
DEFAULT = re.compile(r'env\("([A-Z0-9_]+)",\s*"([^"]*)"\)|env\("([A-Z0-9_]+)"\)\s*\.unwrap_or_else\(\|\| "([^"]*)"\.into\(\)\)')
COMPOSE_DEFAULT = re.compile(r"\$\{([A-Z][A-Z0-9_]*):-([^}]*)\}")
# A table of the page: its name, and its rows' names and defaults.
TABLE = re.compile(r"export const (\w+): Var\[\] = \[(.*?)\n\];", re.S)
ROW = re.compile(r"name: '([^']+)', default: (?:null|'([^']*)')")


def main():
    code = {n for d in SOURCES for f in sorted((ROOT / d).rglob("*.rs")) for n in NAME.findall(f.read_text())}
    code |= {n for f in COMPOSE for n in re.findall(r"\$\{([A-Z][A-Z0-9_]*)", (ROOT / f).read_text())}
    # Comments go first, so a commented-out row is never read as one.
    page = re.sub(r"/\*.*?\*/", "", (ROOT / TABLES).read_text(), flags=re.S)
    page = re.sub(r"^\s*//.*$", "", page, flags=re.M)
    if len(ROW.findall(page)) != page.count("{ name: "):
        sys.exit(f"{TABLES}: a row is not `{{ name: '…', default: null | '…', … }}`")
    listed = {n.strip() for names, _ in ROW.findall(page) for n in names.split(",")}
    errors = [f"{TABLES} lists no {n}, which the code reads" for n in sorted(code - listed)]
    errors += [f"{TABLES} lists {n}, which no code reads" for n in sorted(listed - code - LIBRARY)]
    # The defaults the code states, by name: the services' own, and Compose's (the page's COMPOSE table).
    stated, composed = {}, {}
    for d in SOURCES:
        for f in sorted((ROOT / d).rglob("*.rs")):
            for a, b, c, e in DEFAULT.findall(f.read_text()):
                stated.setdefault(a or c, set()).add(b or e)
    for f in COMPOSE:
        for n, value in COMPOSE_DEFAULT.findall((ROOT / f).read_text()):
            composed.setdefault(n, set()).add(value)
    for table, rows in TABLE.findall(page):
        for name, default in ROW.findall(rows):
            code_default = (composed if table == "COMPOSE" else stated).get(name)
            if code_default and default not in code_default:
                errors.append(f"{TABLES}: {table} gives {name} the default {default!r}; the code's is "
                              f"{' or '.join(sorted(code_default))}")
    if errors:
        sys.exit("\n".join(errors))
    print(f"{len(listed)} settings listed, each one the code reads")


if __name__ == "__main__":
    main()
