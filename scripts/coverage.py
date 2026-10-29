#!/usr/bin/env python3
"""Totals the line coverage scripts/coverage.sh measured, from DIR/rust.lcov and DIR/client.lcov:

    scripts/coverage.py DIR

Prints each one's lines covered and found, and writes their total to DIR/coverage.json as a shields.io endpoint (the
README's badge). LLVM counts a Rust unit test module's lines like any others, so a `#[cfg(test)]` item's lines are
left out of the totals (rust.lcov keeps them, as LLVM wrote it): the percentage is of the code under test only."""

import json
import sys
from pathlib import Path

# The badge's colour from the percentage, highest first; anything lower is orange.
COLOURS = ((80, "brightgreen"), (70, "green"), (60, "yellowgreen"), (50, "yellow"))


def records(path):
    """Each source file in an lcov file with its lines' hit counts, summed over repeated entries."""
    files, source = {}, None
    for line in path.read_text().splitlines():
        if line.startswith("SF:"):
            source = files.setdefault(line[3:], {})
        elif line.startswith("DA:"):
            number, hits = line[3:].split(",")[:2]
            source[int(number)] = source.get(int(number), 0) + int(hits)
    return files


def test_lines(text):
    """The line numbers of each `#[cfg(test)]` item, with its attributes, in rustfmt's layout. Its head runs to the
    first line ending in `{` or `;` (or the item's own line, when it ends in `,`, as a field does); a head ending in `{`
    runs on to the line that is only its closing brace, at the item's indentation, whatever lines inside are indented
    less (a raw string's)."""
    lines, found = text.splitlines(), set()
    for start, line in enumerate(lines):
        if line.strip() != "#[cfg(test)]":
            continue
        pad = line[: len(line) - len(line.lstrip())]
        end = start + 1
        while end < len(lines) and lines[end].strip().startswith("#["):
            end += 1
        if not lines[end].rstrip().endswith(","):
            while end < len(lines) - 1 and not lines[end].rstrip().endswith(("{", ";")):
                end += 1
        if lines[end].rstrip().endswith("{"):
            # Its closing brace alone on a line at its own indentation: not `} else {`, nor `}}"#` ending a raw string.
            end = next((after for after in range(end + 1, len(lines)) if lines[after].rstrip() == pad + "}"), len(lines) - 1)
        found.update(range(start + 1, end + 2))
    return found


def total(files):
    return sum(len(hits) for hits in files.values()), sum(1 for hits in files.values() for n in hits.values() if n)


def main():
    out = Path(sys.argv[1])
    rust = records(out / "rust.lcov")
    for source, hits in rust.items():
        for number in test_lines(Path(source).read_text()):
            hits.pop(number, None)
    client = records(out / "client.lcov")
    found, covered = 0, 0
    for name, files in (("rust", rust), ("client", client)):
        lines, hit = total(files)
        found, covered = found + lines, covered + hit
        print(f"{name}: {hit} of {lines} lines covered ({100 * hit / lines:.1f}%)")
    print(f"total: {covered} of {found} lines covered ({100 * covered / found:.1f}%)")
    # Rounded down, so the badge never claims more than was measured.
    percent = 100 * covered // found
    colour = next((c for floor, c in COLOURS if percent >= floor), "orange")
    badge = {"schemaVersion": 1, "label": "coverage", "message": f"{percent}%", "color": colour}
    (out / "coverage.json").write_text(json.dumps(badge) + "\n")


if __name__ == "__main__":
    main()
