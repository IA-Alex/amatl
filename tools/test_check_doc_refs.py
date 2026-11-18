#!/usr/bin/env python3
"""Mutation/property tests for the integer-argument half of check_doc_refs.py.

The gate already rejects a `--flag` the command does not declare. It did not
look at the *value*: `amatl db downgrade --to four` names a real flag on a real
subcommand and reads as an instruction the operator cannot run.

These cases pin the clean corpus, a non-integer value on a known numeric flag,
a numeric flag with no value at all, and a non-numeric flag whose value must
stay free-form (`--config-file x.toml`).
"""

import subprocess
import sys
from pathlib import Path

REPOSITORY_ROOT = Path(__file__).resolve().parent.parent
PROBE = REPOSITORY_ROOT / "docs" / "_gate_probe.md"


def run_with(lines):
    PROBE.write_text("\n".join(["# probe", "", "```bash", *lines, "```", ""]), encoding="utf-8")
    try:
        r = subprocess.run(
            [sys.executable, str(REPOSITORY_ROOT / "tools/check_doc_refs.py")],
            cwd=REPOSITORY_ROOT,
            capture_output=True,
            text=True,
        )
        return r.stdout + r.stderr
    finally:
        PROBE.unlink()


CASES = (
    ("amatl db downgrade --to 4", False, "valid integer"),
    ("amatl db downgrade --to four", True, "non-integer on a numeric flag"),
    ("amatl history list --limit 20", False, "valid integer"),
    ("amatl history list --limit veinte", True, "non-integer on a numeric flag"),
    ("amatl history list --limit", True, "numeric flag with no value"),
    ("amatl deep --max-fetches 5", False, "valid integer"),
    ("amatl deep --max-fetches many", True, "non-integer on a numeric flag"),
    ("amatl search --config-file x.toml --json", False, "free-form flag stays free"),
)


def main():
    failures = 0
    for cmd, should_flag, label in CASES:
        out = run_with([cmd])
        flagged = "_gate_probe.md" in out and "dead command" in out
        ok = flagged == should_flag
        failures += 0 if ok else 1
        print(f"{'ok ' if ok else 'FAIL'}: [{label}] {cmd!r} flagged={flagged} expected={should_flag}")
        if not ok:
            for line in out.splitlines():
                if "_gate_probe" in line:
                    print("      ", line[:160])
    print(f"\n{len(CASES) - failures}/{len(CASES)} passed")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
