#!/usr/bin/env python3
"""Generate/refresh SPDX + AI-disclosure headers from git history.

For every tracked source file (Rust, Python, Shell, Nix, Makefile) this tool
walks the file's git history (following renames), samples the human authors and
the AI models recorded in commit trailers, and writes a uniform header block.

Schema (mirrors flake.nix):

    # Copyright (c) 2026 <human authors, one per line>
    #
    # This program and the accompanying materials are made available under
    # the terms of the Eclipse Public License 2.0 which accompanies this
    # distribution, and is available at https://www.eclipse.org/legal/epl-2.0/
    #
    # AI Disclosure: This file was mostly AI-generated.
    #
    # SPDX-License-Identifier: EPL-2.0 and CC0-1.0
    # Assisted-by: <normalized models>

Files whose history traces back to the "prework" commit are never touched.

Usage:
    python3 tools/license_headers.py --dry-run
    python3 tools/license_headers.py --apply
"""

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# Commit that introduced the challenge's pre-work (demo/scaffolding). Files that
# can be followed back to it are off-limits.
PREWORK_COMMITS = {"5bd2391"}

# The schema reference itself is kept verbatim.
SKIP_FILES = {"flake.nix"}

TARGET_RE = re.compile(r"\.(rs|py|sh|nix)$")
RECORD_SEP = "\x1e"
FIELD_SEP = "\x1f"

TRAILER_RE = re.compile(
    r"^(?:Assisted-by|Assited-by|Authored-by|Co-Authored-By):\s*(.+?)\s*$",
    re.IGNORECASE | re.MULTILINE,
)

# Normalization of the model spellings found in commit trailers.
MODEL_PATTERNS: list[tuple[re.Pattern[str], str]] = [
    (re.compile(r"deepseek[-\s]?v?4\.1[-\s]?flash", re.IGNORECASE), "DeepSeek v4.1 Flash"),
    (re.compile(r"(?:z\.ai\s+|any[-\s])?glm[-\s]?5\.3[-\s]?flash", re.IGNORECASE), "GLM-5.3-flash"),
    (re.compile(r"xiaomi\s+mimo[-\s]?v?2\.6[-\s]?pro", re.IGNORECASE), "Xiaomi MiMo v2.6 Pro"),
    (re.compile(r"claude\s+opus\s+5\.5", re.IGNORECASE), "Claude Opus 5.5"),
    (re.compile(r"claude\s+sonnet\s+5\.5", re.IGNORECASE), "Claude Sonnet 5.5"),
]

BOILERPLATE = [
    "This program and the accompanying materials are made available under",
    "the terms of the Eclipse Public License 2.0 which accompanies this",
    "distribution, and is available at https://www.eclipse.org/legal/epl-2.0/",
]
AI_DISCLOSURE = "AI Disclosure: This file was mostly AI-generated."
SPDX_LINE = "SPDX-License-Identifier: EPL-2.0 and CC0-1.0"


def git(*args: str) -> str:
    return subprocess.run(
        ["git", "-C", REPO, *args],
        check=True,
        capture_output=True,
        text=True,
    ).stdout


def tracked_files() -> list[str]:
    out = git("ls-files")
    files = []
    for line in out.splitlines():
        if not line:
            continue
        if os.path.basename(line) == "Makefile" or TARGET_RE.search(line):
            files.append(line)
    return files


def first_add(path: str) -> str:
    out = git("log", "--diff-filter=A", "--follow", "--format=%h", "--", path)
    lines = [line for line in out.splitlines() if line]
    return lines[-1] if lines else ""


def history(path: str) -> list[dict[str, object]]:
    fmt = f"%an{FIELD_SEP}%B{RECORD_SEP}"
    out = git("log", "--follow", "--no-merges", f"--format={fmt}", "--", path)
    records = []
    for record in out.split(RECORD_SEP):
        record = record.strip("\n")
        if not record:
            continue
        if FIELD_SEP not in record:
            continue
        author, body = record.split(FIELD_SEP, 1)
        records.append({"author": author.strip(), "body": body})
    return records


def normalize_model(raw: str) -> list[str]:
    models = []
    for part in re.split(r"\s+and\s+|,", raw):
        part = part.strip()
        if not part:
            continue
        for pattern, canonical in MODEL_PATTERNS:
            if pattern.search(part):
                if canonical not in models:
                    models.append(canonical)
                break
    return models


def attribution(path: str) -> tuple[list[str], list[str]]:
    authors: list[str] = []
    models: list[str] = []
    # git log prints newest first; reverse for chronological copyright order.
    for rec in reversed(history(path)):
        author = str(rec["author"])
        if author and author not in authors:
            authors.append(author)
        for raw in TRAILER_RE.findall(str(rec["body"])):
            for model in normalize_model(raw):
                if model not in models:
                    models.append(model)
    return authors, models


def comment_prefix(path: str) -> str:
    if path.endswith(".rs"):
        return "//"
    return "#"


def build_block(path: str, authors: list[str], models: list[str]) -> list[str]:
    p = comment_prefix(path)
    block = [f"{p} Copyright (c) 2026 {author}" for author in authors]
    block.append(p)
    block.extend(f"{p} {line}" for line in BOILERPLATE)
    block.append(p)
    block.append(f"{p} {AI_DISCLOSURE}")
    block.append(p)
    block.append(f"{p} {SPDX_LINE}")
    if models:
        block.append(f"{p} Assisted-by: {', '.join(models)}")
    return block


def rewrite(path: str, block: list[str]) -> None:
    abspath = os.path.join(REPO, path)
    with open(abspath, "r", encoding="utf-8") as handle:
        content = handle.read()
    lines = content.split("\n")
    prefix = comment_prefix(path)

    start = 1 if lines and lines[0].startswith("#!") else 0

    has_managed = any(
        "SPDX-License-Identifier" in lines[i]
        for i in range(start, min(len(lines), start + 40))
    )

    if has_managed:
        spdx_i = next(
            i
            for i in range(start, min(len(lines), start + 40))
            if "SPDX-License-Identifier" in lines[i]
        )
        block_end = spdx_i + 1
        # A managed header may end with a single Assisted-by line; anything
        # after that is the file's own content and must be preserved.
        if (
            block_end < len(lines)
            and lines[block_end].lstrip().startswith(prefix)
            and "Assisted-by:" in lines[block_end]
        ):
            block_end += 1
        new_lines = lines[:start] + block + lines[block_end:]
    else:
        new_lines = lines[:start] + block + lines[start:]

    with open(abspath, "w", encoding="utf-8") as handle:
        handle.write("\n".join(new_lines))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--dry-run", action="store_true", help="print planned headers")
    mode.add_argument("--apply", action="store_true", help="write the headers")
    args = parser.parse_args()

    processed = skipped = 0
    for path in tracked_files():
        if path in SKIP_FILES:
            print(f"SKIP (reference)   {path}")
            skipped += 1
            continue
        if first_add(path) in PREWORK_COMMITS:
            print(f"SKIP (pre-work)    {path}")
            skipped += 1
            continue
        authors, models = attribution(path)
        block = build_block(path, authors, models)
        if args.dry_run:
            print(f"=== {path} ===")
            print("\n".join(block))
            print()
        else:
            rewrite(path, block)
            print(f"WROTE {path}  ({', '.join(authors) or 'no author'})"
                  f"  models: {', '.join(models) or '-'}")
        processed += 1

    print(f"\n{processed} file(s) {'planned' if args.dry_run else 'updated'}, "
          f"{skipped} skipped.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
