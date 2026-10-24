#!/usr/bin/env python3
"""Writes the README's comparison tables from the landing's own comparison (site/src/lib/content.ts).

    scripts/compare.py           # rewrite the tables between the README's compare markers
    scripts/compare.py --check   # write nothing; fail when the README or docs/comparison.md differ from the landing

One table of every criterion, easi.run first and then the rivals in COLUMNS: a criterion is marked ✅ where the landing
marks it yes, and – otherwise (no, partly, not in their docs, or not applicable), as the landing does; under it, how
many rivals the full comparison covers. The check also reads docs/comparison.md, whose verdict for each rival and
criterion (`- **Criterion:** Yes…`) must be the landing's. scripts/check.sh runs the check.
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CONTENT = ROOT / "site" / "src" / "lib" / "content.ts"
README = ROOT / "README.md"
NOTES = ROOT / "docs" / "comparison.md"
# The verdict words docs/comparison.md opens a criterion with, as the landing's cells (y, p, n, u, na); longer first.
# "Yes means" opens what a criterion means, not a verdict.
VERDICTS = (("Yes means", None), ("Not documented", "u"), ("Not applicable", "na"), ("N/A", "na"), ("Partial", "p"), ("Partly", "p"),
            ("Unknown", "u"), ("Yes", "y"), ("No", "n"))
# A rival named one way in docs/comparison.md and another on the landing.
ALIASES = {"xpra-html5": "xpra"}
# The criteria docs/comparison.md describes beside the landing's rows, with no verdict to check.
CONTEXT = {"Live view", "Saved logins", "Saved logins (context)"}
START, END = "<!-- compare:start -->", "<!-- compare:end -->"
# The rivals the README sets beside easi.run: a cloud browser, a cloud browser with a self-hosted image, and VNC.
COLUMNS = ("Browserbase", "Browserless", "KasmVNC")
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


def tables():
    rows, ours, rivals = comparison()
    by_name = {r["product"]: r for r in rivals}
    if not set(COLUMNS) <= by_name.keys():
        sys.exit(f"site/src/lib/content.ts: no {sorted(set(COLUMNS) - by_name.keys())} among the rivals")
    mark = {"y": "✅"}
    out = ["| | **easi.run** | " + " | ".join(COLUMNS) + " |", "|---|" + ":---:|" * (len(COLUMNS) + 1)]
    for key, label in rows:
        cells = [mark.get(ours[key], "–")] + [mark.get(by_name[name]["cells"][key], "–") for name in COLUMNS]
        out.append(f"| {label} | " + " | ".join(cells) + " |")
    out += ["", "✅ has it · – doesn't, partly, not in their docs, or not applicable. The "
            "[full comparison](https://easi.run/#compare)",
            f"covers {len(rivals)} alternatives, with [the sources and notes](docs/comparison.md) (October 2026)."]
    return "\n".join(out) + "\n"


def notes():
    """Where docs/comparison.md's verdict for easi.run or a rival on a criterion is not the landing's."""
    rows, ours, rivals = comparison()
    keys = {label: key for key, label in rows}
    by_name = {r["product"]: r["cells"] for r in rivals} | {"easi.run": ours}
    wrong = []
    for section in re.split(r"^### ", NOTES.read_text(), flags=re.M)[1:]:
        head, _, body = section.partition("\n")
        name = ALIASES.get(head.strip(), head.strip())
        if name not in by_name:
            continue
        for label, verdict in re.findall(r"^- \*\*([^*]+):\*\* (.*)$", body, re.M):
            if label in CONTEXT:
                continue
            if label not in keys:
                wrong.append(f"docs/comparison.md: {name}: {label!r} is no criterion of the landing's")
                continue
            said = next((cell for word, cell in VERDICTS if verdict.startswith(word)), "?")
            if said == "?":
                wrong.append(f"docs/comparison.md: {name}, {label}: {verdict[:40]!r} opens with no verdict word")
            elif said is not None and said != by_name[name][keys[label]]:
                wrong.append(f"docs/comparison.md: {name}, {label}: {verdict[:40]!r} is not the landing's "
                             f"{by_name[name][keys[label]]}")
    return wrong


def main():
    readme = README.read_text()
    if START not in readme or END not in readme:
        sys.exit(f"README.md: no {START} … {END} block")
    before, rest = readme.split(START, 1)
    _, after = rest.split(END, 1)
    written = f"{before}{START}\n\n{tables()}\n{END}{after}"
    if "--check" in sys.argv[1:]:
        errors = notes()
        if written != readme:
            errors.append("README.md: the comparison tables differ from site/src/lib/content.ts; run scripts/compare.py")
        if errors:
            sys.exit("\n".join(errors))
        print("the README's comparison and docs/comparison.md's verdicts match the landing's")
        return
    README.write_text(written)


if __name__ == "__main__":
    main()
