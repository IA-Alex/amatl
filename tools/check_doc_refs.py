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

The same drift hits the *commands* the guides tell an operator to run. A
`amatl <subcommand>` written in a fenced code block is an instruction, and when
the CLI renames or drops the subcommand the guide keeps telling the operator to
run something that no longer exists -- the reader finds out by typing it and
reading the error. This gate closes that half too: every `amatl …` invocation
inside a code block must name a subcommand the CLI declares, checked against
`CLI_SUBCOMMANDS` below. Only fenced code blocks are inspected: `amatl` in prose
("AMATL ships…", "the amatl binary…") is not an instruction and stays out.

Like the reference half, it is *declaration* checking, not execution: the table
below mirrors the `clap` definitions by hand, the gate runs no binary and needs
no build. Renaming a subcommand means updating this table in the same change --
which is the point, because that is the moment the guides drift.

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

Both halves run together; either failing returns non-zero.
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

# The subcommands the CLI declares, as `{name: ()}` for a leaf command and
# `{name: (child, …)}` for one that has subcommands. A hand-kept mirror of the
# `clap` definitions in `crates/amatl-cli/src/main.rs`; keeping it beside the
# gate is what turns "rename a subcommand" into a change the reviewer sees next
# to the guide update. Top-level entries come from `amatl --help`; nested
# entries from `<command> --help`. `help` is clap's implicit subcommand and is
# present on every level, so it is not listed.
CLI_SUBCOMMANDS = {
    "search": (),
    "deep": (),
    "ingest": (),
    "providers": (),
    "provider-canary": (),
    "config": (),
    "cache": (),
    "doctor": (),
    "benchmark": (),
    "history": ("list", "delete", "purge"),
    "saved": ("list", "show", "delete"),
    "db": (
        "health",
        "backups",
        "restore",
        "downgrade",
        "security-events",
        "circuits",
    ),
    "serve": (),
    "mcp": ("serve",),
}

# The long flags each command declares, keyed by the same path the invocation
# uses: a bare command (`"deep"`), a nested one (`"db downgrade"`). A hand-kept
# mirror of the `clap` definitions, same reasoning as CLI_SUBCOMMANDS -- a
# guide that tells the operator to pass `--to` to a command that dropped it is
# the same class of rot. `--config-file` is on every command and `--help`/
# `--version` are clap built-ins, so none is listed; a cited `--flag` must
# appear in the entry for the command it is attached to.
CLI_FLAGS = {
    "search": ("json",),
    "deep": ("json", "max-fetches"),
    "ingest": ("query", "json"),
    "providers": (),
    "provider-canary": ("json",),
    "config": (),
    "cache": ("purge",),
    "doctor": (),
    "benchmark": ("json", "iterations", "concurrency"),
    "history": (),
    "history list": ("limit", "offset", "json"),
    "history delete": (),
    "history purge": (),
    "saved": (),
    "saved list": ("limit", "offset", "json"),
    "saved show": (),
    "saved delete": (),
    "db": (),
    "db health": ("json",),
    "db backups": (),
    "db restore": (),
    "db downgrade": ("to",),
    "db security-events": ("limit", "offset", "json"),
    "db circuits": ("reset", "json"),
    "serve": ("bind", "port", "json"),
    "mcp": (),
    "mcp serve": ("bind", "port", "json"),
}

# Long flags that clap provides on every command, so citing them is never rot.
CLI_UNIVERSAL_FLAGS = ("config-file", "help", "version")

# A `--long-flag` in an invocation line, excluding the `--` separator and
# `--flag=value` (the name is captured either way).
CLI_FLAG = re.compile(r"(?<![\w-])--([a-z][a-z0-9-]*)")

# Flags whose value `clap` parses as an integer (`<LIMIT>`, `<TO>`, …). A guide
# that writes `--limit veinte` names a real flag on a real subcommand, so the
# flag check passed, yet the operator cannot run it. `--config-file x.toml` is
# deliberately absent: its value is any path, and pinning a type there would
# reject correct documentation.
CLI_INTEGER_FLAGS = (
    "limit",
    "offset",
    "to",
    "iterations",
    "concurrency",
    "max-fetches",
)

# `--flag VALUE` or `--flag=VALUE` in an invocation line.
CLI_FLAG_VALUE = re.compile(r"(?<![\w-])--([a-z][a-z0-9-]*)(?:=(\S+)|(?:\s+(\S+)))?")

# An `amatl …` invocation inside a fenced code block. Leading shell noise is
# allowed (`$`, a prompt, or `VAR=value` assignments) so the common
# `AMATL_SERVER_TOKEN=… amatl serve` form still matches. Captures the first bare
# word after `amatl`, which is the subcommand position.
CLI_INVOCATION = re.compile(
    r"^\s*(?:[A-Za-z_][A-Za-z0-9_]*=\S+\s+|\$\s+|#\s+|>\s+)*amatl\s+([a-z][a-z0-9-]*)"
)

# A second word that is a nested subcommand or a leaf argument. Lowercase words
# are compared against the parent's children; anything else (a path, a quoted
# string, a flag) is an argument and is not a subcommand position.
CLI_SECOND_WORD = re.compile(r"^\s*amatl\s+[a-z][a-z0-9-]*\s+([a-z][a-z0-9-]*)\b")

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
    # Cargo's own `readme` warning, quoted verbatim in the C4 finding of
    # `hallazgos-2026-10-26.md`. The path inside it is relative to the crate
    # root, not to the repository, so the resolver -- which only knows
    # repository-relative paths -- can never resolve it. The quote is the
    # evidence the finding rests on, so it stays verbatim instead of being
    # rewritten into something the gate can parse.
    ("docs/security/hallazgos-2026-10-26.md", "../../README.md"),
    # The 2026-10-26 findings report cites the SearXNG adapter while discussing
    # `Budget::new(…, 1_000)` call sites. That adapter was later removed from the
    # product (the retirement ADR), so the citation is dead — but the report is a
    # dated record of what was true at the time, and rewriting its code citations
    # would falsify that record. The reference stays verbatim.
    (
        "docs/security/hallazgos-2026-10-26.md",
        "crates/amatl-core/src/providers/searxng.rs:391",
    ),
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


def code_blocks(document):
    """`(line_number, text)` for every line inside a fenced code block."""
    inside = False
    for number, line in enumerate(
        document.read_text(encoding="utf-8", errors="replace").splitlines(), start=1
    ):
        if line.strip().startswith("```"):
            inside = not inside
            continue
        if inside:
            yield number, line


def command_violations():
    """`(document, where, reason)` for every `amatl …` invocation the CLI rejects.

    Two classes: a subcommand the CLI does not declare, and a long flag the
    command it is attached to does not declare. Both are read from fenced code
    blocks only -- an `amatl …` in prose is not an instruction.
    """
    found = []
    for document in scanned_documents():
        relative = str(document.relative_to(REPOSITORY_ROOT))
        for number, line in code_blocks(document):
            first = CLI_INVOCATION.match(line)
            if not first:
                continue
            subcommand = first.group(1)
            if subcommand not in CLI_SUBCOMMANDS:
                found.append(
                    (
                        relative,
                        f"{relative}:{number}: amatl {subcommand}",
                        f"no such subcommand: amatl {subcommand}",
                    )
                )
                continue
            # Resolve the command path (`db downgrade` vs `deep`) and the flag
            # set it is checked against.
            path = subcommand
            children = CLI_SUBCOMMANDS[subcommand]
            second = CLI_SECOND_WORD.match(line)
            if children and second and second.group(1) in children:
                path = f"{subcommand} {second.group(1)}"
            elif children and second and second.group(1) != "help":
                nested = second.group(1)
                found.append(
                    (
                        relative,
                        f"{relative}:{number}: amatl {subcommand} {nested}",
                        f"no such subcommand: amatl {subcommand} {nested}",
                    )
                )
                continue
            elif children and not second:
                # A command with children cited bare (`amatl db`): clap prints
                # help, which is not rot, so nothing more to check.
                continue
            declared = CLI_FLAGS.get(path, ())
            for flag in CLI_FLAG.findall(line):
                if flag in CLI_UNIVERSAL_FLAGS or flag in declared:
                    continue
                found.append(
                    (
                        relative,
                        f"{relative}:{number}: amatl {path} --{flag}",
                        f"no such flag: --{flag} on amatl {path}",
                    )
                )
            # Un flag numérico debe llevar un número. `--limit veinte` nombra un
            # flag real y por eso pasaba el chequeo anterior, pero no es una
            # instrucción ejecutable.
            for match in CLI_FLAG_VALUE.finditer(line):
                flag = match.group(1)
                value = match.group(2) if match.group(2) is not None else match.group(3)
                if flag not in CLI_INTEGER_FLAGS:
                    continue
                if value is not None and value.lstrip("-").isdigit():
                    continue
                shown = f" {value}" if value is not None else ""
                found.append(
                    (
                        relative,
                        f"{relative}:{number}: amatl {path} --{flag}{shown}",
                        f"--{flag} on amatl {path} takes an integer"
                        + ("" if value is not None else ", and none was given"),
                    )
                )
    return found


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
    dead_commands = command_violations()

    for _, _, where, reason in new:
        print(f"dead reference {where} ({reason})")
    for _, where, reason in dead_commands:
        print(f"dead command {where} ({reason})")
    if "--list" in argv:
        for document, reference in sorted(FROZEN):
            print(f"frozen: {document}: {reference}")

    failed = False
    if new:
        print(
            f"{len(new)} dead reference(s). Fix the path or the line, or freeze "
            "the pair in FROZEN with a reason if the document deliberately "
            "cites something that no longer exists."
        )
        failed = True
    if dead_commands:
        print(
            f"{len(dead_commands)} dead command(s). Fix the `amatl <subcommand>` "
            "in the guide, or update CLI_SUBCOMMANDS if the CLI itself changed."
        )
        failed = True
    if failed:
        return 1
    print(f"doc references: {len(found)} known-dead, {len(new)} new")
    print(f"doc commands: {len(dead_commands)} dead")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
