#!/usr/bin/env python3
"""Writes the README's comparison tables from the landing's own comparison (site/src/lib/content.ts).

    scripts/compare.py           # rewrite the tables between the README's compare markers
    scripts/compare.py --check   # write nothing; fail when the README's tables differ from the landing's comparison

One table per group of rivals (cloud browsers, then self-hosted ones), easi.run first: a criterion is marked ✅ where
the landing marks it yes, and – otherwise (no, partly, not in their docs, or not applicable), as the landing does.
scripts/check.sh runs the check.
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CONTENT = ROOT / "site" / "src" / "lib" / "content.ts"
README = ROOT / "README.md"
START, END = "<!-- compare:start -->", "<!-- compare:end -->"
GROUPS = (("cloud", "Against cloud browsers for agents"), ("selfhosted", "Against self-hosted remote browsers"))
CELL = re.compile(r"^\s+(\w+): (y|p|n|u|na)\(", re.M)


def cells(block, where):
    """A block's cells by row, each found once (a row given twice is refused, not silently overridden)."""
    found = CELL.findall(block)
    keys = [key for key, _ in found]
    if len(keys) != len(set(keys)):
        sys.exit(f"site/src/lib/content.ts: {where} gives a row twice")
    return dict(found)


def comparison():
    # Comments go first, so a commented-out cell is never read as one.
    text = re.sub(r"/\*.*?\*/", "", CONTENT.read_text(), flags=re.S)
    text = re.sub(r"^\s*//.*$", "", text, flags=re.M)
    rows = re.findall(r"\{ key: '(\w+)', label: '([^']*)' \}", text)
    ours = text[text.index("export const OURS"):text.index("export const RIVALS")]
    rivals = []
    for block in text[text.index("export const RIVALS"):].split("product: '")[1:]:
        rivals.append({
            "product": block[:block.index("'")],
            "logo": re.search(r"logo: '([^']+)'", block).group(1),
            "group": re.search(r"group: '(\w+)'", block).group(1),
            "cells": cells(block, block[:block.index("'")]),
        })
    ours = cells(ours, "OURS")
    if not rows or not rivals or ours.keys() != {key for key, _ in rows}:
        sys.exit("site/src/lib/content.ts: the comparison's rows, ours or rivals are not as expected")
    for rival in rivals:
        if rival["cells"].keys() != {key for key, _ in rows}:
            sys.exit(f"site/src/lib/content.ts: {rival['product']} lacks a cell for some row")
    return rows, ours, rivals


def head(name, logo):
    return f'<img src="{logo}" height="20" alt=""><br>{name}'


def tables():
    rows, ours, rivals = comparison()
    mark = {"y": "✅"}
    out = []
    for group, title in GROUPS:
        members = [r for r in rivals if r["group"] == group]
        out.append(f"#### {title}\n")
        heads = [head("**easi.run**", "site/public/favicon.svg")]
        heads += [head(r["product"], f"site/public/logos/{r['logo']}") for r in members]
        out.append("| | " + " | ".join(heads) + " |")
        out.append("|---|" + ":---:|" * len(heads))
        for key, label in rows:
            cells = [mark.get(ours[key], "–")] + [mark.get(r["cells"][key], "–") for r in members]
            out.append(f"| {label} | " + " | ".join(cells) + " |")
        out.append("")
    return "\n".join(out)


def main():
    readme = README.read_text()
    if START not in readme or END not in readme:
        sys.exit(f"README.md: no {START} … {END} block")
    before, rest = readme.split(START, 1)
    _, after = rest.split(END, 1)
    written = f"{before}{START}\n\n{tables()}\n{END}{after}"
    if "--check" in sys.argv[1:]:
        if written != readme:
            sys.exit("README.md: the comparison tables differ from site/src/lib/content.ts; run scripts/compare.py")
        print("the README's comparison matches the landing's")
        return
    README.write_text(written)


if __name__ == "__main__":
    main()
