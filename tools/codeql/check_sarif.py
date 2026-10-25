#!/usr/bin/env python3
"""Fail when an AMATL CodeQL invariant query reports a violation.

`tools/codeql/queries/` holds *violation* queries: an empty result set means the
invariant holds. `codeql database analyze` writes SARIF and exits 0 either way,
so this turns "how many results came back" into an exit code.

    codeql database analyze <db> tools/codeql \\
      --format=sarif-latest --output=/tmp/amatl-invariants.sarif
    python3 tools/codeql/check_sarif.py /tmp/amatl-invariants.sarif
"""

import json
import sys
from pathlib import Path


def violations(sarif_path):
    """`(location, rule, description)` for every result in the SARIF file."""
    sarif = json.loads(Path(sarif_path).read_text(encoding="utf-8"))
    found = []
    for run in sarif.get("runs", []):
        driver = run.get("tool", {}).get("driver", {})
        descriptions = {
            rule.get("id", "?"): rule.get("shortDescription", {}).get("text", "")
            for rule in driver.get("rules", [])
        }
        for result in run.get("results", []):
            rule = result.get("ruleId", "?")
            locations = result.get("locations", [])
            if locations:
                physical = locations[0].get("physicalLocation", {})
                artifact = physical.get("artifactLocation", {}).get("uri", "?")
                line = physical.get("region", {}).get("startLine", "?")
                location = f"{artifact}:{line}"
            else:
                location = "?"
            found.append((location, rule, descriptions.get(rule, "")))
    return found


def main(argv):
    if len(argv) != 1:
        print(__doc__.strip())
        return 2
    found = violations(argv[0])
    for location, rule, description in found:
        print(f"{location}: {rule} -> {description}")
    if found:
        print(f"{len(found)} invariant violation(s)")
        return 1
    print("no invariant violations")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
