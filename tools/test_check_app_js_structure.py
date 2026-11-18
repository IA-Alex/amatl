#!/usr/bin/env python3
"""Mutation tests for tools/check_app_js_structure.py.

A structural checker that always prints `ok` is worse than none: it is a green
light nobody watches. These cases inject each defect the checker claims to
cover, require it to fail, and require the clean file to pass. The clean case
also guards the other direction, since every earlier version of this checker
reported a false positive on a function parameter or a `return` inside a helper.

Run from anywhere:

    python3 tools/test_check_app_js_structure.py
"""

import subprocess
import sys
from pathlib import Path

REPOSITORY_ROOT = Path(__file__).resolve().parent.parent
APP = REPOSITORY_ROOT / "crates/amatl-ui/assets/app.js"
CHECKER = REPOSITORY_ROOT / "tools/check_app_js_structure.py"


def run_checker():
    result = subprocess.run(
        [sys.executable, str(CHECKER)], cwd=REPOSITORY_ROOT, capture_output=True, text=True
    )
    return result.returncode, (result.stdout + result.stderr).strip()


def with_mutation(transform):
    original = APP.read_text(encoding="utf-8")
    APP.write_text(transform(original), encoding="utf-8")
    try:
        return run_checker()
    finally:
        APP.write_text(original, encoding="utf-8")


# A `const` read from top-level code runs while the file loads. `deepItems` is
# declared far below, so a call at the top is exactly the temporal-dead-zone
# read this check exists for.
def use_before_definition(text):
    return text.replace('"use strict";', '"use strict";\ndeepItems({});', 1)


# `renderClientRow` is declared inside the `clients` IIFE and not returned, so
# naming it outside the IIFE is a load-time ReferenceError.
def private_leak(text):
    return text.replace("refreshPanels();", "renderClientRow(null);\nrefreshPanels();", 1)


CASES = (
    ("clean file passes", None, False),
    ("load-time use before definition", use_before_definition, True),
    ("private name referenced outside its IIFE", private_leak, True),
)


def main():
    failures = 0
    for label, transform, should_flag in CASES:
        code, output = run_checker() if transform is None else with_mutation(transform)
        flagged = code != 0
        ok = flagged == should_flag
        failures += 0 if ok else 1
        state = "ok" if ok else "FAIL"
        print(f"{state}: [{label}] flagged={flagged} expected={should_flag}")
        if not ok:
            print(f"      {output.splitlines()[0][:160] if output else '(no output)'}")
    print(f"\n{len(CASES) - failures}/{len(CASES)} passed")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
