#!/usr/bin/env python3
"""Writes the READMEs' comparison tables from the landing's own comparison (site/src/lib/content.ts, its verdicts, and
site/messages/<locale>/compare.json, its rows' names in each language): README.md's in English, and each other
language's README, docs/readme/<locale>.md, in its own.

    scripts/compare.py           # rewrite the tables between each README's compare markers
    scripts/compare.py --check   # write nothing; fail when a README or docs/comparison.md differ from the landing

One table of every criterion, easi.run first and then the rivals in COLUMNS: a criterion is marked ✅ where the landing
marks it yes, and – otherwise (no, partly, not in their docs, or not applicable), as the landing does; under it, how
many rivals the full comparison covers. The check also reads docs/comparison.md, whose verdict for each rival and
criterion (`- **Criterion:** Yes…`) must be the landing's. scripts/check.sh runs the check.
"""

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CONTENT = ROOT / "site" / "src" / "lib" / "content.ts"
MESSAGES = ROOT / "site" / "messages"
LOCALES = ROOT / "i18n" / "locales.ts"
NOTES = ROOT / "docs" / "comparison.md"
# The verdict words docs/comparison.md opens a criterion with, as the landing's cells (y, p, n, u, na); longer first.
# "Yes means" opens what a criterion means, not a verdict.
VERDICTS = (("Yes means", None), ("Not documented", "u"), ("Not applicable", "na"), ("Partial", "p"), ("Unknown", "u"),
            ("Yes", "y"), ("No", "n"))
# A rival named one way in docs/comparison.md and another on the landing.
ALIASES = {"xpra-html5": "xpra"}
# The criterion docs/comparison.md describes beside the landing's rows, with no verdict to check.
CONTEXT = {"Live view"}
START, END = "<!-- compare:start -->", "<!-- compare:end -->"
# The rivals the README sets beside easi.run: a cloud browser, a cloud browser with a self-hosted image, and VNC.
COLUMNS = ("Browserbase", "Browserless", "KasmVNC")
CELL = re.compile(r"^\s+(\w+): (y|p|n|u|na)\(", re.M)
# The line under each language's table: {compare} is the full comparison's address, {notes} the sources' path from that
# README, {count} how many rivals it covers.
LEGEND = {
    "en": "✅ has it · – doesn't, partly, not in their docs, or not applicable. The [full comparison]({compare})\n"
          "covers {count} alternatives, with [the sources and notes]({notes}) (October 2026).",
    "pt-BR": "✅ tem · – não tem, tem em parte, não está na documentação deles, ou não se aplica. A\n"
             "[comparação completa]({compare}) cobre {count} alternativas, com [as fontes e notas]({notes}) (outubro de 2026).",
    "es": "✅ lo tiene · – no lo tiene, lo tiene en parte, no está en su documentación, o no aplica. La\n"
          "[comparación completa]({compare}) cubre {count} alternativas, con [las fuentes y notas]({notes}) (octubre de 2026).",
    "zh-CN": "✅ 有 · – 没有、部分支持、文档中未提及或不适用。[完整对比]({compare})涵盖 {count} 个替代方案，\n"
             "并附有[来源和说明]({notes})（2026 年 10 月）。",
    "hi": "✅ है · – नहीं है, आंशिक है, उनके डॉक्स में नहीं है, या लागू नहीं। [पूरी तुलना]({compare})\n"
          "{count} विकल्पों को कवर करती है, [स्रोतों और नोट्स]({notes}) के साथ (अक्टूबर 2026)।",
    "fr": "✅ l'a · – ne l'a pas, en partie, absent de leur documentation, ou sans objet. La\n"
          "[comparaison complète]({compare}) couvre {count} alternatives, avec [les sources et les notes]({notes}) (octobre 2026).",
    "de": "✅ vorhanden · – nicht vorhanden, teilweise, nicht in ihrer Doku oder nicht zutreffend. Der\n"
          "[vollständige Vergleich]({compare}) umfasst {count} Alternativen, mit [Quellen und Notizen]({notes}) (Oktober 2026).",
    "ja": "✅ あり · – なし、一部のみ、ドキュメントに記載なし、または該当なし。[完全な比較]({compare})では\n"
          "{count} の代替製品を、[出典と注記]({notes})付きで取り上げています（2026 年 10 月）。",
    "ru": "✅ есть · – нет, частично, нет в их документации или неприменимо. [Полное сравнение]({compare})\n"
          "охватывает {count} альтернатив, с [источниками и примечаниями]({notes}) (октябрь 2026).",
    "id": "✅ punya · – tidak punya, sebagian, tidak ada di dokumentasi mereka, atau tidak berlaku. [Perbandingan\n"
          "lengkap]({compare}) mencakup {count} alternatif, dengan [sumber dan catatannya]({notes}) (Oktober 2026).",
    "ko": "✅ 있음 · – 없음, 일부만, 문서에 없음 또는 해당 없음. [전체 비교]({compare})는 {count}개 대안을\n"
          "[출처와 메모]({notes})와 함께 다룹니다(2026년 10월).",
}


def cells(block, where):
    """A block's cells by row, each found once (a row given twice is refused, not silently overridden)."""
    found = CELL.findall(block)
    keys = [key for key, _ in found]
    if len(keys) != len(set(keys)):
        sys.exit(f"site/src/lib/content.ts: {where} gives a row twice")
    return dict(found)


def locales():
    """The languages easi.run speaks, English first (i18n/locales.ts's AUTONYMS)."""
    text = LOCALES.read_text()
    block = text[text.index("AUTONYMS = {"):text.index("};")]
    return re.findall(r"^\s+'?([\w-]+)'?: '", block, re.M)


def readme(locale):
    return ROOT / "README.md" if locale == "en" else ROOT / "docs" / "readme" / f"{locale}.md"


def comparison(locale="en"):
    # Comments go first, so a commented-out cell is never read as one.
    text = re.sub(r"/\*.*?\*/", "", CONTENT.read_text(), flags=re.S)
    text = re.sub(r"^\s*//.*$", "", text, flags=re.M)
    names = json.loads((MESSAGES / locale / "compare.json").read_text())
    listed = text[text.index("export const ROWS"):]
    keys = re.findall(r"^\s+'(\w+)',$", listed[:listed.index("] as const")], re.M)
    missing = [key for key in keys if f"row.{key}" not in names]
    if missing:
        sys.exit(f"site/messages/{locale}/compare.json: no row.{missing[0]}")
    rows = [(key, names[f"row.{key}"]) for key in keys]
    ours = text[text.index("export const OURS"):text.index("export const RIVALS")]
    rivals = []
    for block in text[text.index("export const RIVALS"):].split("product: '")[1:]:
        rivals.append({
            "product": block[:block.index("'")],
            "cells": cells(block, block[:block.index("'")]),
        })
    ours = cells(ours, "OURS")
    if not rows or not rivals or ours.keys() != {key for key, _ in rows}:
        sys.exit("site/src/lib/content.ts: the comparison's rows, ours or rivals are not as expected")
    for rival in rivals:
        if rival["cells"].keys() != {key for key, _ in rows}:
            sys.exit(f"site/src/lib/content.ts: {rival['product']} lacks a cell for some row")
    return rows, ours, rivals


def tables(locale):
    rows, ours, rivals = comparison(locale)
    by_name = {r["product"]: r for r in rivals}
    if not set(COLUMNS) <= by_name.keys():
        sys.exit(f"site/src/lib/content.ts: no {sorted(set(COLUMNS) - by_name.keys())} among the rivals")
    mark = {"y": "✅"}
    out = ["| | **easi.run** | " + " | ".join(COLUMNS) + " |", "|---|" + ":---:|" * (len(COLUMNS) + 1)]
    for key, label in rows:
        cells = [mark.get(ours[key], "–")] + [mark.get(by_name[name]["cells"][key], "–") for name in COLUMNS]
        out.append(f"| {label} | " + " | ".join(cells) + " |")
    home = "" if locale == "en" else f"{locale}/"
    notes = "docs/comparison.md" if locale == "en" else "../comparison.md"
    out += ["", LEGEND[locale].format(compare=f"https://easi.run/{home}compare/", count=len(rivals), notes=notes)]
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


def written(locale):
    """A README as it is, and as it is with its tables written from the landing's."""
    path = readme(locale)
    where = path.relative_to(ROOT)
    if not path.exists():
        sys.exit(f"{where}: missing; every language easi.run speaks has its README")
    text = path.read_text()
    if START not in text or END not in text:
        sys.exit(f"{where}: no {START} … {END} block")
    before, rest = text.split(START, 1)
    _, after = rest.split(END, 1)
    return path, text, f"{before}{START}\n\n{tables(locale)}\n{END}{after}"


def main():
    readmes = [written(locale) for locale in locales()]
    if "--check" in sys.argv[1:]:
        errors = notes()
        errors += [f"{path.relative_to(ROOT)}: the comparison tables differ from site/src/lib/content.ts; run "
                   "scripts/compare.py" for path, text, new in readmes if new != text]
        if errors:
            sys.exit("\n".join(errors))
        print("the READMEs' comparisons and docs/comparison.md's verdicts match the landing's")
        return
    for path, text, new in readmes:
        if new != text:
            path.write_text(new)


if __name__ == "__main__":
    main()
