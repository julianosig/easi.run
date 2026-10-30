#!/usr/bin/env python3
"""Checks that every list of the public demo's hosts names the same demos: ADMIN_DEMO_SITES in demo/compose.yaml
(each at https://NAME.play/), both host patterns in demo/nginx.conf, the certificate's names in demo/sites-cert.sh,
the pages in client/src/ui/demos.ts, the landing's tab names (site/messages/en/demo.json, `NAME.title` and
`NAME.line`), and the directories under demo/sites/ (besides `shared/`). A demo missing from one of them gets a
certificate error, is sent home, or shows no title (on the landing, an English one).

Checks too that the demo speaks the same languages everywhere: i18n/locales.ts's, which the client's door sends, those
admin/src/demo.rs accepts, those demo/sites/shared/words.js picks among, and those of the demo pages' words (English
in words.js, the others in words.json beside it), each language with English's words, of the same kinds and with the
same {placeholders}. A language the admin lacks refuses the visit; a word a language lacks shows in English."""

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


def languages():
    """Each list of the languages the demo speaks, by where it is, and the demo pages' words by language."""
    locales = (ROOT / "i18n" / "locales.ts").read_text()
    block = re.search(r"^export const AUTONYMS = \{\n(.*?)^\};", locales, re.M | re.S)
    if block is None:
        sys.exit("i18n/locales.ts: no AUTONYMS table")
    accepted = one(r"^const LOCALES: \[&str; \d+\] = \[(.*)\];$", (ROOT / "admin" / "src" / "demo.rs").read_text(),
                   "admin/src/demo.rs")
    shared = DEMO / "sites" / "shared"
    script = (shared / "words.js").read_text()
    english = re.search(r"^const ENGLISH = (\{\n.*?^\});$", script, re.M | re.S)
    if english is None:
        sys.exit("demo/sites/shared/words.js: no ENGLISH table")
    spoken = one(r"^const LOCALES = \[(.*)\];$", script, "demo/sites/shared/words.js")
    words = {"en": json.loads(english.group(1)), **json.loads((shared / "words.json").read_text())}
    return {
        "i18n/locales.ts AUTONYMS": re.findall(r"^  '?([A-Za-z-]+)'?:", block.group(1), re.M),
        "admin/src/demo.rs LOCALES": re.findall(r'"([^"]+)"', accepted),
        "demo/sites/shared/words.js LOCALES": re.findall(r"'([^']+)'", spoken),
        "demo/sites/shared/words.js and words.json": list(words),
    }, words


def differences(found, what):
    """Where each list of `found` names one twice or lacks one another names."""
    every = set().union(*found.values())
    failures = []
    for where, names in found.items():
        if len(names) != len(set(names)):
            failures.append(f"{where}: {what} listed twice: {names}")
        if missing := sorted(every - set(names)):
            failures.append(f"{where}: missing {', '.join(missing)}")
    return every, failures


def shape(word):
    """What a word must keep in every language: its kind, a list's length, a text's placeholders."""
    if isinstance(word, str):
        return "text", sorted(re.findall(r"\{\w+\}", word))
    return type(word).__name__, len(word) if isinstance(word, list) else None


def unlike_english(words):
    """Each word of a language that English lacks, lacks of English's, or has of another shape."""
    english = words.get("en", {})
    failures = []
    for language, table in words.items():
        for key in sorted(english.keys() | table.keys()):
            if key not in table or key not in english:
                failures.append(f"demo/sites/shared/words.json: {language} {'lacks' if key in english else 'adds'} {key}")
            elif shape(table[key]) != shape(english[key]):
                failures.append(f"demo/sites/shared/words.json: {language} {key} is not shaped as English's")
    return failures


def main():
    found = lists()
    every, failures = differences(found, "a demo")
    spoken, words = languages()
    tongues, unlike = differences(spoken, "a language")
    failures += unlike + unlike_english(words)
    if failures:
        print("the demo's lists differ:", *failures, sep="\n  ")
        sys.exit(1)
    print(f"the demo's {len(found)} host lists name the same {len(every)} demos, and its {len(spoken)} language lists"
          f" the same {len(tongues)} languages, each with every word")


if __name__ == "__main__":
    main()
