#!/usr/bin/env python3
"""Fail on dead `file:line` references in the documentation.

The security documents cite code as `` `path/to/file.rs:120-140` `` so a reader
can jump straight to the control a claim rests on. When the cited file is
renamed or split -- for example `config.rs` into `config/mod.rs` plus
`config/server.rs` -- the reference silently stops resolving: nothing in the
build reads the documentation, so the drift is only found by a human following
the link, and a checklist that cites a file that no longer exists reads as
evidence while being none.

This gate closes the *dead-reference* half of that problem: every reference must
name a file that exists in the repository, and every line number must lie inside
that file. It cannot detect *semantic* drift -- a reference that still resolves
but now points at unrelated code -- so a reference must still be re-checked
whenever the code it cites moves.

Bare names (`fetch.rs`, `config/mod.rs`) are resolved by suffix, because the
documents write them relative to the crate or module that owns them; a reference
that matches several files is accepted when at least one of them satisfies the
line numbers. A reference written under a name the repository only carries in
another form -- the deployed `amatl.toml` against the shipped
`amatl.example.toml` -- is resolved through `ALIASES`.

Pre-existing dead references are frozen in `FROZEN` below, so this gate blocks
*new* ones only. Each entry is a debt note: fix the reference and delete its
line.

Run from anywhere:

    python3 tools/check_doc_refs.py           # non-zero on new dead references
    python3 tools/check_doc_refs.py --list    # also list the frozen baseline
"""

import re
import sys
from pathlib import Path

REPOSITORY_ROOT = Path(__file__).resolve().parent.parent

# Directories that carry documentation. `decisiones_amatl.md` and
# `CONTINUIDAD.md` are deliberately absent: they are historical records, and
# rewriting their references would falsify what was true when the decision was
# taken.
SCANNED = ("docs", "README.md", "SECURITY.md")

# Only paths that look like source or documentation files are references; this
# keeps requirement ids (`v5.0.0-2.2.1`) and prose out of the gate.
EXTENSIONS = (
    "rs",
    "toml",
    "md",
    "yaml",
    "yml",
    "sql",
    "js",
    "css",
    "html",
    "py",
    "sh",
)

REFERENCE = re.compile(
    r"`([A-Za-z0-9_./-]+\.(?:"
    + "|".join(EXTENSIONS)
    + r"))((?::[0-9]+(?:-[0-9]+)?(?:,[0-9]+(?:-[0-9]+)?)*))?`"
)

# References that are intentionally dead, as `(document, reference)` pairs. Each
# entry is a debt note with a reason; the gate fails on anything else.
FROZEN = {
    # The historical path this very paragraph describes: the split of
    # `config.rs` into `config/` is the thing being documented, so the reference
    # to the old file is the point of the sentence, not a stale citation.
    ("docs/security/asvs-checklist.md", "crates/amatl-core/src/config.rs"),
    # Upstream `candle-core` source cited while discussing its zip handling; the
    # file lives in the dependency, not in this repository.
    ("docs/security/supply-chain.md", "src/npy.rs"),
    # The STEP 4C/4D candle experiment records: the stack they describe was
    # quarantined out of the workspace (RUSTSEC-2026-0367/-0368/-0369 through
    # `zip < 9.0.0`) into `experiments/local-embeddings-boundary/`, a crate
    # excluded from the workspace. The measurements and the in-workspace layout
    # they cite are what was true when they were written, and each document
    # carries a status note naming where the code lives now, so rewriting the
    # paths would falsify the record.
    (
        "docs/experiments/candle-e2e-4c.md",
        "crates/amatl-core/src/experimental_embeddings/mod.rs",
    ),
    (
        "docs/experiments/candle-e2e-4c.md",
        "crates/amatl-core/tests/experimental_embedding_latency.rs",
    ),
    ("docs/experiments/candle-integration-4d.md", "experimental_embeddings/mod.rs"),
    ("docs/experiments/candle-integration-4d.md", "experimental_embeddings/candle.rs"),
    (
        "docs/experiments/candle-integration-4d.md",
        "experimental_embeddings/model_config.rs",
    ),
    (
        "docs/experiments/candle-integration-4d.md",
        "experimental_embeddings/pipeline.rs",
    ),
    # Host-side SearXNG files captured during a campaign: the SearXNG install
    # and a rollback snapshot under /tmp, neither of which the repository owns.
    ("docs/testing/searxng-characterization.md", "/etc/searxng/settings.yml"),
    ("docs/testing/searxng-reproducible-campaign-closure.md", "settings.yml"),
    ("docs/testing/searxng-reproducible-campaign-closure.md", "/tmp/settings_now.yml"),
}


def repository_files():
    """Every file the repository owns, excluding build output and VCS metadata."""
    excluded = {".git", "target", "node_modules", "__pycache__"}
    files = []
    for path in REPOSITORY_ROOT.rglob("*"):
        if not path.is_file():
            continue
        relative = path.relative_to(REPOSITORY_ROOT)
        if excluded.intersection(relative.parts):
            continue
        files.append(relative)
    return files


def scanned_documents():
    """The Markdown documents under `SCANNED`."""
    documents = []
    for entry in SCANNED:
        target = REPOSITORY_ROOT / entry
        if target.is_dir():
            documents.extend(sorted(target.rglob("*.md")))
        elif target.is_file():
            documents.append(target)
    return documents


def spans(suffix):
    """The `line` / `line-line` / `line,line` groups of a reference suffix."""
    for part in suffix.lstrip(":").split(","):
        start, _, end = part.partition("-")
        yield int(start), int(end) if end else int(start)


# References written under a name the repository does not carry. The documents
# cite the operator's configuration file as `amatl.toml`, which is the name it
# has once the operator copies the template into place -- and `.gitignore` keeps
# that copy out of the repository, so a fresh checkout only ever has
# `amatl.example.toml`. Resolving the pair keeps those citations resolvable --
# and line-checked wherever a line is cited -- against the template instead of
# freezing them, and keeps the gate meaningful in CI, which never has the
# deployed file.
ALIASES = {
    "amatl.toml": "amatl.example.toml",
}


def candidates(reference, index):
    """Files the reference could name: an exact path, or a suffix match."""
    exact = [path for path in index if str(path) == reference]
    if exact:
        return exact
    matches = [
        path
        for path in index
        if str(path).endswith("/" + reference) or path.name == reference
    ]
    if matches:
        return matches
    alias = ALIASES.get(reference)
    return [path for path in index if str(path) == alias] if alias else []


def line_count(path):
    text = (REPOSITORY_ROOT / path).read_text(encoding="utf-8", errors="replace")
    return len(text.splitlines())


def resolves(suffix, matches):
    """Whether at least one candidate file satisfies the reference's lines."""
    if not suffix:
        return True
    return any(
        all(1 <= start <= end <= line_count(path) for start, end in spans(suffix))
        for path in matches
    )


def violations(index):
    """`(document, reference, where, reason)` for every unresolvable reference."""
    found = []
    for document in scanned_documents():
        relative_document = str(document.relative_to(REPOSITORY_ROOT))
        text = document.read_text(encoding="utf-8", errors="replace")
        for line_number, line in enumerate(text.splitlines(), start=1):
            for match in REFERENCE.finditer(line):
                reference = f"{match.group(1)}{match.group(2) or ''}"
                matches = candidates(match.group(1), index)
                reason = None
                if not matches:
                    reason = "no such file"
                elif not resolves(match.group(2), matches):
                    reason = "line past the end of the file"
                if reason:
                    where = f"{relative_document}:{line_number}: {reference}"
                    found.append((relative_document, reference, where, reason))
    return found


def main(argv):
    index = repository_files()
    found = violations(index)
    new = [entry for entry in found if (entry[0], entry[1]) not in FROZEN]

    for _, _, where, reason in new:
        print(f"dead reference {where} ({reason})")
    if "--list" in argv:
        for document, reference in sorted(FROZEN):
            print(f"frozen: {document}: {reference}")

    if new:
        print(
            f"{len(new)} dead reference(s). Fix the path or the line, or freeze "
            "the pair in FROZEN with a reason if the document deliberately "
            "cites something that no longer exists."
        )
        return 1
    print(f"doc references: {len(found)} known-dead, {len(new)} new")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
