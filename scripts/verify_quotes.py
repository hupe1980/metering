#!/usr/bin/env python3
"""Check every passage the crate presents as verbatim source text against the PDFs.

Reads `src/**/*.rs`, `site/content/docs/*.md` and `README.md`, pulls out every
`*"…"*` passage, and searches a corpus built with `pdftotext -layout` over
`specs/` (plus any extra directories given on the command line).

A quote is verified when every fragment of it — the passage split on `…`, since
the crate elides — appears in the corpus after normalisation. Normalisation
strips the things a PDF-to-text pass and a doc comment disagree about: markdown
emphasis, escaped brackets, soft hyphens, line-break hyphenation, typographic
quotes and dashes, and non-breaking spaces.

Exit code is 1 when a quote that is expected to verify does not.

    python3 scripts/verify_quotes.py [extra-pdf-dir ...]
"""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# Quotes whose source is not held as searchable text. Each is verified by hand
# against the published document; the reason it cannot be verified mechanically
# is stated, because "not found" and "not checkable" are different answers.
UNVERIFIABLE = {
    "a b c d e werden im deutschen energiemarkt verwendet": (
        "a label inside a figure — pdftotext cannot see it; the neighbouring "
        "§ 2.3 Wertegruppe F passage does verify"
    ),
}

# `*"…"*` and `**"…"**` alike: a quote does not stop being a quote because it
# is set in bold.
QUOTE = re.compile(r'[*_]{1,2}"(.+?)"[*_]{1,2}', re.S)

# Anything German enough to be a citation rather than English emphasis.
GERMAN = re.compile(
    r"[äöüßÄÖÜ]|\b(?:der|die|das|den|dem|des|eine|einer|einem|einen|und|oder|"
    r"nicht|wird|werden|wurde|sind|ist|vom|zur|zum|dass|durch|sowie|soweit|"
    r"jeweils|deutschen|nur|auch|einer|keine|kann|muss|soll)\b",
    re.I,
)


def normalise(text: str) -> str:
    text = text.replace("­", "")            # soft hyphen
    text = text.replace(" ", " ")           # NBSP
    text = re.sub(r"-\s*\n\s*", "", text)        # hyphenation across a line break
    # A hyphen between two lower-case letters is typesetting, not spelling:
    # `-layout` keeps "Korrekturenergie-mengen" on one line where the PDF broke
    # it. Removed on both sides, so a real compound — whose hyphen is followed
    # by a space or a capital — is untouched.
    text = re.sub(r"(?<=[a-zäöüß])-(?=[a-zäöüß])", "", text)
    text = text.replace("„", '"').replace("“", '"').replace("”", '"')
    text = text.replace("’", "'").replace("‘", "'")
    text = text.replace("–", "-").replace("—", "-").replace("−", "-")
    text = re.sub(r"[*_`]", "", text)
    text = text.replace("\\[", "[").replace("\\]", "]")
    # A bracketed ellipsis is the scholarly form of the same elision the bare
    # one marks, and both are split on below.
    text = re.sub(r"\[\s*(?:…|\.\.\.)\s*\]", "…", text)
    text = text.replace("...", "…")
    text = re.sub(r"^\s*(///|//!|//)\s?", "", text, flags=re.M)
    # Case-folded: a passage quoted mid-sentence starts lower case where the
    # document starts a sentence, and that difference is never a misquote.
    return re.sub(r"\s+", " ", text).strip().lower()


def corpus(dirs: list[Path]) -> str:
    parts: list[str] = []
    for directory in dirs:
        for pdf in sorted(directory.rglob("*.pdf")):
            out = subprocess.run(
                ["pdftotext", "-layout", str(pdf), "-"],
                capture_output=True,
                text=True,
                check=False,
            )
            parts.append(out.stdout)
    return normalise("\n".join(parts))


def sources() -> list[Path]:
    files = sorted((ROOT / "src").rglob("*.rs"))
    files += sorted((ROOT / "site" / "content" / "docs").glob("*.md"))
    files.append(ROOT / "README.md")
    return [f for f in files if f.exists()]


def main() -> int:
    dirs = [ROOT / "specs"] + [Path(a).expanduser() for a in sys.argv[1:]]
    dirs = [d for d in dirs if d.is_dir()]
    if not dirs:
        print("no PDF directory found — run `just specs` first", file=sys.stderr)
        return 1

    haystack = corpus(dirs)
    if not haystack:
        print("empty corpus — is pdftotext installed?", file=sys.stderr)
        return 1

    checked = verified = skipped = 0
    failures: list[tuple[str, str]] = []

    for path in sources():
        text = path.read_text(encoding="utf-8")
        for raw in QUOTE.findall(text):
            quote = normalise(raw)
            # German only: the crate quotes its sources in their own language,
            # and an English *"…"* is emphasis, not a citation. The marker list
            # is wide on purpose — a quote that slips past it is a quote nobody
            # checks, which is worse than an English string reported once.
            if not GERMAN.search(quote):
                continue
            # Fewer than four words is a term, not a citation — a holiday's
            # name, a register label. Emphasis, not a claim about a document.
            if len(quote.split(" ")) < 4:
                continue
            checked += 1
            if any(marker in quote for marker in UNVERIFIABLE):
                skipped += 1
                continue
            fragments = [f.strip() for f in quote.split("…") if len(f.strip()) > 12]
            if fragments and all(f in haystack for f in fragments):
                verified += 1
            else:
                missing = next((f for f in fragments if f not in haystack), quote)
                failures.append((str(path.relative_to(ROOT)), missing))

    print(f"{checked} German passages: {verified} verified, {skipped} known-unverifiable, "
          f"{len(failures)} unmatched")
    for path, fragment in failures:
        print(f"  {path}\n    {fragment[:160]}")
    if failures:
        print(
            "\nAn unmatched passage is a misquote until proven otherwise. Either the "
            "wording is wrong, or the source is not in the corpus — add its directory "
            "as an argument, or record it in UNVERIFIABLE with the reason.",
            file=sys.stderr,
        )
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
