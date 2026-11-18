#!/usr/bin/env bash
# Advisory gate: `cargo audit` fails on the whole lockfile, `cargo deny` re-checks
# it against `deny.toml`. Both run, and the suppressed IDs are re-derived from
# `deny.toml` on every invocation so a suppression cannot grow without showing up
# in the log. `cargo audit` receives the same IDs through its own flag (its config
# file is not `deny.toml`); one source feeds both, so they cannot diverge.
#
# Usage: scripts/security-gate.sh [--expected-ignore RUSTSEC-XXXX-XXXX]...
#
# Without `--expected-ignore` the gate reports the suppressed IDs. With one or
# more, the set found in `deny.toml` must match exactly: adding an `ignore` entry
# then also requires changing the CI invocation, which is the review checkpoint a
# blindly red gate used to provide for every advisory at once.
#
# `cargo audit --deny warnings` is deliberately not set: `cargo deny` classifies
# `unsound` and `yanked` and fails on them, so the two gates still disagree on
# severity instead of one silently overriding the other.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

expected=()
while [ "$#" -gt 0 ]; do
    case "$1" in
        --expected-ignore)
            expected+=("${2:?--expected-ignore needs an advisory ID}")
            shift 2
            ;;
        *)
            echo "security-gate: unknown argument: $1" >&2
            exit 2
            ;;
    esac
done

# The IDs listed under `ignore` in the `[advisories]` table of `deny.toml`.
ignored_ids() {
    awk '
        /^\[/ { section = $0; next }
        section == "[advisories]" && /^ignore[[:space:]]*=/ { in_list = 1 }
        in_list {
            line = $0
            while (match(line, /"RUSTSEC-[0-9]{4}-[0-9]{4}"/)) {
                print substr(line, RSTART + 1, RLENGTH - 2)
                line = substr(line, RSTART + RLENGTH)
            }
            if (line ~ /\]/) in_list = 0
        }
    ' deny.toml | sort -u
}

actual="$(ignored_ids)"

if [ "${#expected[@]}" -gt 0 ]; then
    wanted="$(printf '%s\n' "${expected[@]}" | sort -u)"
    if [ "$actual" != "$wanted" ]; then
        echo "security-gate: suppressed advisories drifted from the reviewed set" >&2
        echo "  expected:" >&2
        printf '    %s\n' "$wanted" >&2
        echo "  found in deny.toml:" >&2
        printf '    %s\n' "${actual:-<none>}" >&2
        exit 1
    fi
fi

if [ -n "$actual" ]; then
    echo "security-gate: suppressed advisories (each needs an ADR and a threat model):"
    printf '  %s\n' "$actual"
else
    echo "security-gate: no suppressed advisories"
fi

audit_args=()
while IFS= read -r id; do
    [ -n "$id" ] && audit_args+=(--ignore "$id")
done <<<"$actual"

cargo audit "${audit_args[@]}"
cargo deny check
