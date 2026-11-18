#!/usr/bin/env python3
"""Fail on the two structural defects in `app.js` that the tests cannot see.

The asset tests in `crates/amatl-ui/src/lib.rs` assert on string presence and
never execute JavaScript. That is enough for the copy and the markup, and it is
blind to the two ways a reorganised `app.js` breaks at load while staying green:

1. **Use before definition at load time.** A `const` read from top-level code
   that runs while the file loads is a temporal-dead-zone `ReferenceError`. The
   page never renders. A use inside a function body is fine -- it runs later --
   which is why this check counts only top-level depth.

2. **A private name referenced outside its IIFE.** Each domain is
   `const d = (function () { ... return { a, b }; })();`. A function declared
   inside and *not* returned is private; naming it outside the IIFE throws at
   load, and a name passed by reference (`addEventListener("click", fn)`)
   escapes a scan that only looks for calls.

Both classes shipped during the 2026-11-18 refactor and both passed
`cargo test`. The first was caught by a browser; the second only by this check.

Run from anywhere:

    python3 tools/check_app_js_structure.py

Exit status is non-zero on any finding. Standard library only: no binary, no
network, nothing to install, the same contract as `tools/check_doc_refs.py`.
"""

import re
import sys
from pathlib import Path

REPOSITORY_ROOT = Path(__file__).resolve().parent.parent
APP = REPOSITORY_ROOT / "crates/amatl-ui/assets/app.js"

# `function name(` at any indentation, with an optional `async`.
DECLARATION = re.compile(r"^\s*(?:async )?function ([A-Za-z_][A-Za-z0-9_]*)\s*\(", re.M)
# `const X = (function () {` opens a domain IIFE.
IIFE = re.compile(r"^const ([A-Za-z_][A-Za-z0-9_]*) = \(function \(\) \{")
# The IIFE closes on a line that is exactly `})();`.
IIFE_CLOSE = "})();"
# `const name = domain.member;` is the legitimate top-level bridge.
ALIAS = re.compile(r"^const [A-Za-z_][A-Za-z0-9_]* = ([A-Za-z_][A-Za-z0-9_]*)\.[A-Za-z_][A-Za-z0-9_]*;")


def load():
    return APP.read_text(encoding="utf-8")


def domain_iifes(lines):
    """`(name, start_index, close_index, body)` for every domain IIFE."""
    spans = []
    for index, line in enumerate(lines):
        match = IIFE.match(line)
        if not match:
            continue
        depth = 0
        for close in range(index, len(lines)):
            depth += lines[close].count("{") - lines[close].count("}")
            if depth == 0 and lines[close].strip() == IIFE_CLOSE:
                spans.append((match.group(1), index, close, lines[index : close + 1]))
                break
    return spans


def load_time_uses(lines):
    """First top-level-depth use of each identifier, ignoring comments.

    Depth is tracked with braces. A line only counts at depth 0 *before* its own
    braces are applied, which is the position where the loader would evaluate
    it.
    """
    uses = {}
    depth = 0
    in_block_comment = False
    for number, line in enumerate(lines, start=1):
        stripped = line.strip()
        if in_block_comment:
            if "*/" in stripped:
                in_block_comment = False
            continue
        if stripped.startswith("/*"):
            if "*/" not in stripped:
                in_block_comment = True
            continue
        if stripped.startswith("//"):
            continue
        if depth == 0:
            # Una declaración de función registra sus parámetros en la misma
            # línea a profundidad 0 (`function renderStatus(render, status) {`),
            # y un parámetro no es un uso del identificador. Se descarta esa
            # línea para no reportar un falso positivo.
            if not DECLARATION.match(stripped):
                for match in re.finditer(r"\b([A-Za-z_][A-Za-z0-9_]*)\b", stripped):
                    uses.setdefault(match.group(1), number)
        depth += line.count("{") - line.count("}")
        if depth < 0:
            depth = 0
    return uses


def use_before_definition(lines):
    """Findings for a `function` read at top level before it is declared."""
    declarations = {}
    for number, line in enumerate(lines, start=1):
        match = DECLARATION.match(line)
        if match:
            declarations.setdefault(match.group(1), number)
    uses = load_time_uses(lines)
    findings = []
    for name, declared_at in declarations.items():
        used_at = uses.get(name)
        # The declaration line itself registers as a use; anything at or before
        # it is not a load-time read of the function.
        if used_at is not None and used_at < declared_at:
            findings.append(
                f"{name}: read at load (line {used_at}) before its declaration (line {declared_at})"
            )
    return findings


def private_leaks(lines):
    """Findings for a name private to an IIFE that is used outside it."""
    findings = []
    for name, start, close, body in domain_iifes(lines):
        text = "".join(body)
        declared = set(DECLARATION.findall(text))
        # The IIFE's own return is the last `return { ... };` in its body.
        returns = [i for i, line in enumerate(body) if re.search(r"return \{[^}]*\};", line)]
        if not returns:
            findings.append(f"{name}: domain IIFE has no return object")
            continue
        exposed = set()
        for token in re.search(r"return \{([^}]*)\};", body[returns[-1]]).group(1).split(","):
            token = token.strip()
            if re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", token):
                exposed.add(token)
        private = declared - exposed
        outside = "".join(lines[:start]) + "".join(lines[close + 1 :])
        # `const x = domain.member;` is the bridge and is not a leak.
        outside = "\n".join(line for line in outside.splitlines() if not ALIAS.match(line.strip()))
        for function_name in sorted(private):
            if re.search(rf"\b{function_name}\b", outside):
                findings.append(
                    f"{function_name}: private to IIFE '{name}' (opens at line {start + 1}) "
                    "but referenced outside it"
                )
    return findings


def main():
    if not APP.is_file():
        print(f"missing asset: {APP}")
        return 1
    lines = load().splitlines(keepends=True)
    found = [f"use-before-definition: {f}" for f in use_before_definition(lines)]
    found += [f"private-leak: {f}" for f in private_leaks(lines)]
    for finding in found:
        print(finding)
    if found:
        print(
            f"{len(found)} structural defect(s) in app.js. Fix the order or return "
            "the symbol from its IIFE; both fail at load while cargo test stays green."
        )
        return 1
    print("app.js structure: ok (no load-time use before definition, no private leaks)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
