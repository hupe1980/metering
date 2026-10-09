#!/usr/bin/env python3
"""Hold `concepts/` and `specs/` to the rules `concepts/README.md` states.

Both folders are gitignored and never ship, so no test in `tests/` can read
them and `cargo doc` never sees them. This is the maintainer's equivalent of
`zola check`, run as `just _concepts-check`. A missing `concepts/` is a fault —
a check that passes by finding nothing has checked nothing; `--allow-missing`
is for a checkout that deliberately has none.

This docstring is the one home of the rules.

1. **Every cross-file link resolves**, file and anchor. An anchor pointing at a
   renamed heading lands the reader somewhere plausible, which is worse than
   dangling. Links from a specification into `concepts/` are held too.
2. **A decision or risk number resolves.** `D17` names a `### D17` heading in
   DECISIONS.md and `R16` a `### R16` heading in RISKS.md — in `concepts/` and
   in each `spec.md`; plans and research number their own findings. The numbers are
   stable identifiers cited across documents; one that names nothing is a
   citation of nothing.
3. **Open work lives in ROADMAP.md only.** Every other document describes
   current state.
4. **A code reference resolves to code.** `Type::member` and `module::item` in
   backticks name something `src/` defines. Scoped to names this crate has, so
   another specification's vocabulary is skipped without an exemption list.
   DECISIONS.md is not read for this rule: a decision names the surface it
   removed or refused, and that name is meant to resolve to nothing. Of the
   specifications only `spec.md` is read: a plan, its tasks and contracts name
   code that does not exist yet, which is what they are for. A `Type::member`
   is checked only when the crate itself defines `Type` — `Decimal::…` is
   another crate's vocabulary.
5. **Every open item is specified, and every specification is open.** Each
   `###` item in ROADMAP.md carries one `**Specified as:**` naming a folder
   directly under `specs/` and one `**Discharge:**`; each such folder is named by
   exactly one item, and a discharged one lives in `specs/archive/`.
"""

from __future__ import annotations

import pathlib
import re
import sys

REPO = pathlib.Path(__file__).resolve().parent.parent
ROOT = REPO / "concepts"
SPECS = REPO / "specs"
SPEC_DIR = re.compile(r"^\d{3}-[a-z0-9-]+$")

# Phrases that mean "not done" outside the one document allowed to say so.
UNFINISHED = re.compile(
    r"\bTODO\b|\bTBD\b|\bFIXME\b|\bnot yet implemented\b|\bwe should\b|\bstill to do\b",
    re.I,
)


def slug(heading: str) -> str:
    """GitHub's anchor for a heading, near enough for this folder's needs.

    Punctuation is *removed* rather than replaced, which is why an em-dash
    surrounded by spaces yields two hyphens — the spaces survive it.
    """
    s = re.sub(r"`", "", heading.strip().lower())
    s = re.sub(r"[^\w \-]", "", s)
    return s.replace(" ", "-")


def anchors(text: str) -> set[str]:
    return {slug(m.group(1)) for m in re.finditer(r"^#{1,6}\s+(.+?)\s*$", text, re.M)}


def line_of(text: str, pos: int) -> int:
    return text[:pos].count("\n") + 1


def main() -> int:
    if not ROOT.is_dir():
        if "--allow-missing" in sys.argv[1:]:
            print(f"no {ROOT} — nothing to check (--allow-missing)")
            return 0
        print(f"no {ROOT}: a check that finds nothing has checked nothing; pass --allow-missing where that is intended")
        return 1
    docs = {p.name: p.read_text() for p in sorted(ROOT.glob("*.md"))}
    if not docs:
        print(f"{ROOT} holds no documents")
        return 1

    specs: dict[str, str] = {}
    if SPECS.is_dir():
        for d in sorted(SPECS.iterdir()):
            if d.is_dir() and SPEC_DIR.match(d.name):
                for md in sorted(d.rglob("*.md")):
                    specs[str(md.relative_to(REPO))] = md.read_text()

    faults: list[str] = []
    by_doc = {name: anchors(text) for name, text in docs.items()}

    # 1. Cross-file links: within concepts/, and from specs/ into concepts/.
    def check_link(origin: str, text: str, pos: int, target: str, frag: str | None) -> None:
        if target not in docs:
            faults.append(f"{origin}:{line_of(text, pos)}: links to {target}, which is not in concepts/")
        elif frag and frag[1:] not in by_doc[target]:
            faults.append(
                f"{origin}:{line_of(text, pos)}: {target}{frag} — no such anchor. A link that "
                f"lands somewhere plausible is worse than one that dangles"
            )

    for name, text in docs.items():
        for m in re.finditer(r"\]\(([A-Z][A-Za-z_]*\.md)(#[^)]+)?\)", text):
            check_link(f"concepts/{name}", text, m.start(), m.group(1), m.group(2))
        for m in re.finditer(r"\]\((#[^)]+)\)", text):
            if m.group(1)[1:] not in by_doc[name]:
                faults.append(f"concepts/{name}:{line_of(text, m.start())}: {m.group(1)} — no such anchor here")
    for path, text in specs.items():
        for m in re.finditer(r"\]\((?:\.\./)+concepts/([A-Z][A-Za-z_]*\.md)(#[^)]+)?\)", text):
            check_link(path, text, m.start(), m.group(1), m.group(2))

    # 2. Decision and risk numbers.
    numbered = {
        "D": set(re.findall(r"^### (D\d+)\b", docs.get("DECISIONS.md", ""), re.M)),
        "R": set(re.findall(r"^### (R\d+)\b", docs.get("RISKS.md", ""), re.M)),
    }
    if not numbered["D"] or not numbered["R"]:
        faults.append("DECISIONS.md or RISKS.md holds no numbered entries — rule 2 would check nothing")
    homes = {"D": "DECISIONS.md", "R": "RISKS.md"}
    # Plans and research number their own findings R1, D-2, …; only the
    # design record and the specifications cite decision and risk numbers.
    spec_texts = {k: v for k, v in specs.items() if k.endswith("/spec.md")}
    for name, text in {**{f"concepts/{k}": v for k, v in docs.items()}, **spec_texts}.items():
        for m in re.finditer(r"(?<![\w-])([DR])(\d{1,3})\b(?![-.]\d)", text):
            ident = m.group(1) + m.group(2)
            if ident not in numbered[m.group(1)]:
                faults.append(
                    f"{name}:{line_of(text, m.start())}: {ident} names no `### {ident}` in "
                    f"{homes[m.group(1)]}"
                )

    # 3. Unfinished work outside the roadmap.
    for name, text in docs.items():
        if name == "ROADMAP.md":
            continue
        for m in UNFINISHED.finditer(text):
            faults.append(
                f"concepts/{name}:{line_of(text, m.start())}: {m.group(0)!r} — open work belongs in "
                f"ROADMAP.md, and every other document describes current state"
            )

    # 4. Code references.
    surface: set[str] = set()
    defined: set[str] = set()
    for rs in sorted((REPO / "src").rglob("*.rs")):
        body = rs.read_text()
        surface.update(re.findall(r"\bfn\s+([a-z_][A-Za-z0-9_]*)", body))
        surface.update(re.findall(r"\b(?:const|static)\s+([A-Z][A-Z0-9_]*)", body))
        surface.update(re.findall(r"^\s*pub\s+([a-z_][A-Za-z0-9_]*)\s*:", body, re.M))
        types = re.findall(r"\b(?:struct|enum|trait|type)\s+([A-Z][A-Za-z0-9]*)", body)
        surface.update(types)
        defined.update(types)
        surface.update(re.findall(r"::([A-Z][A-Za-z0-9]*)", body))
        # Enum variants: a capitalised identifier opening a line inside a body.
        surface.update(re.findall(r"^\s+([A-Z][A-Za-z0-9]*)\s*[,({]", body, re.M))
    modules = {p.stem for p in (REPO / "src").rglob("*.rs")}
    modules |= {d.name for d in (REPO / "src").rglob("*") if d.is_dir()}
    everything = {f"concepts/{k}": v for k, v in docs.items() if k != "DECISIONS.md"}
    everything.update({k: v for k, v in specs.items() if k.endswith("/spec.md")})
    for name, text in everything.items():
        for m in re.finditer(r"`(?:metering::)?([a-z_][a-z0-9_]*)::([a-z_][A-Za-z0-9_]*)`", text):
            module, item = m.group(1), m.group(2)
            if module not in modules or item in surface or item in modules:
                continue
            faults.append(
                f"{name}:{line_of(text, m.start())}: `{module}::{item}` names a module this crate "
                f"has and an item it does not"
            )
        for m in re.finditer(r"`([A-Z][A-Za-z0-9]*)::([A-Za-z_][A-Za-z0-9_]*)", text):
            ty, member = m.group(1), m.group(2)
            if ty not in defined or member in surface:
                continue
            faults.append(
                f"{name}:{line_of(text, m.start())}: `{ty}::{member}` resolves to nothing in src/. "
                f"A design document describing a surface that no longer exists is worse than a "
                f"dangling link: the reader has no reason to doubt it"
            )

    # 5. Open items and their specifications, one to one.
    roadmap = docs.get("ROADMAP.md", "")
    live = {d.name for d in SPECS.iterdir() if d.is_dir() and SPEC_DIR.match(d.name)} if SPECS.is_dir() else set()
    for folder in sorted(live):
        if not (SPECS / folder / "spec.md").is_file():
            faults.append(f"specs/{folder}: no spec.md")
    owners: dict[str, list[str]] = {}
    items = re.findall(r"^### (.+?)\n(.*?)(?=^#{2,3} |\Z)", roadmap, re.M | re.S)
    if not items:
        faults.append("ROADMAP.md: no `###` items — rule 5 would check nothing")
    for heading, body in items:
        named = re.findall(r"^\*\*Specified as:\*\*\s+`([^`]+)`", body, re.M)
        if len(named) != 1:
            faults.append(
                f"ROADMAP.md: {heading!r} names {len(named)} specifications — every open item "
                f"carries exactly one `**Specified as:**` line"
            )
            continue
        if not re.search(r"^\*\*Discharge:\*\*", body, re.M):
            faults.append(f"ROADMAP.md: {heading!r} has no `**Discharge:**` — how would anybody know it is done?")
        owners.setdefault(named[0], []).append(heading)
        if named[0] not in live:
            where = (
                "is archived — a discharged specification's item is deleted, not kept"
                if (SPECS / "archive" / named[0]).is_dir()
                else "does not exist"
            )
            faults.append(f"ROADMAP.md: {heading!r} is specified as {named[0]!r}, which {where}")
    for spec, headings in sorted(owners.items()):
        if len(headings) > 1:
            faults.append(f"ROADMAP.md: {spec!r} is claimed by {len(headings)} items: {headings}")
    for spec in sorted(live - owners.keys()):
        faults.append(
            f"specs/{spec}: no roadmap item names it. Open work outside the roadmap is a second "
            f"backlog; a discharged one moves to specs/archive/"
        )

    print(
        f"{len(docs)} documents, {len(live)} specifications, {len(items)} roadmap items, "
        f"{len(numbered['D'])} decisions, {len(numbered['R'])} risks, {len(surface)} names on the crate surface"
    )
    for fault in faults:
        print(f"  {fault}")
    print(f"{len(faults)} fault(s)")
    return 1 if faults else 0


if __name__ == "__main__":
    sys.exit(main())
