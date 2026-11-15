# Materials audit — 2026-11-15

Advisory gate closure for RUSTSEC-2026-0459, and the three residuals left open by
the `reparador-integral-es` cycle of the same day on items 4 and 5.

## Advisories

| Finding | Decision | Evidence |
|---|---|---|
| RUSTSEC-2026-0459 (`libsqlite3-sys 0.30.1`, fix `>=0.39.0`) | `ignore = ["RUSTSEC-2026-0459"]` in `deny.toml` | `docs/security/supply-chain.md` (threat model), `decisiones_amatl.md` (`ADR-017` exit plan) |

`ignore` is allowed here because the workspace has a written threat model for
this ID and an exit plan, and because every other advisory still fails the gate:
the suppression is one ID, not a policy of tolerating advisories. A blanket red
gate has the opposite effect — it reddens on the known finding every run and the
next, genuinely new advisory arrives in a wall of expected noise.

### Rules the suppression has to keep obeying

1. **One ID, named.** `deny.toml` suppresses one advisory. Adding another needs a
   new ADR and a threat model, and CI's `--expected-ignore` list must change in
   the same commit.
2. **Offline guard.** `crates/amatl-core/tests/supply_chain_contract.rs`
   (`the_audited_sqlite_binding_and_its_suppression_are_still_the_ones_reviewed`)
   fails if the locked `libsqlite3-sys` moves or the suppressed set grows, so a
   reviewer sees it without the RustSec database.
3. **Visibility.** `scripts/security-gate.sh` prints the suppressed IDs on every
   CI run, then runs `cargo audit` with the same IDs and `cargo deny check`.
4. **Pinned tools.** `cargo-audit@0.22.2` and `cargo-deny@0.18.7` are pinned in
   both workflows and the gate is exercised against the pinned `cargo-deny`, so a
   checker that stopped honouring `ignore` would trip the pipeline instead of
   silently passing.

## How to exit the suppression

Change the `sqlx` pin to a line that admits `libsqlite3-sys >= 0.39.0`, then in the
same commit: delete the `ignore` entry, the `--expected-ignore` argument, and the
lockfile assertion added above, and mark `ADR-017` as executed. `sqlx` itself
publishes which feature set `sqlite` implies, so the jump also has to be evaluated
as a packaging change (the `0.9` line adds `sqlite-deserialize` and
`sqlite-load-extension`), not as a version bump.
