# Contributing to AMATL

By participating, you agree to the [Code of Conduct](CODE_OF_CONDUCT.md). Do not
report vulnerabilities in public issues; follow [SECURITY.md](SECURITY.md).

## Workflow

1. Branch from `main` using a short-lived name such as `feat/topic`,
   `fix/topic`, `docs/topic`, or `security/topic`.
2. Keep commits reviewable and use the existing `type: imperative summary`
   convention (`feat`, `fix`, `docs`, `test`, `refactor`, `perf`, `chore`, or
   `security`). Do not mix unrelated changes.
3. Add tests and documentation with the implementation. Open a pull request
   using the repository checklist.
4. Rebase or merge `main` according to the hosting policy before final review;
   no hosting policy is currently encoded, so do not claim a required strategy.

## Protected contracts

Ordinary development must not modify `plan_amatl.md` or
`fase_a_contratos.md`. A proposal to change either starts as an ADR and must
include the violated/changed invariant, alternatives, compatibility and
migration impact, affected Rust/JSON/SQLite/CLI/log names, fixtures, and contract
tests. It may reach the golden template only through a dedicated change with
explicit approval from the repository owner, `@IA-Alex`.

## Definition of done

Every change must compile, preserve one-core ownership, avoid secrets, update
public/configuration docs, and pass the complete local gate. A module below is
not complete without contract tests for valid input, degraded input, typed
error, partial result where applicable, invariants, and exhausted Budget:

- provider: capabilities, filters, availability, typed provider errors, quota;
- canonicalization: safe transformations and degraded ambiguity;
- deduplication: exact merges, provenance and possible duplicates;
- Budget: reservation, exhaustion, deadline and no expansion;
- ranking: signals, bounds, deterministic tie-break and policy version;
- Fetcher: URL, DNS, pinning, redirects, headers, bytes and timeout;
- extractor: missing process, timeout/output bound, invalid output and success;
- router: recommendation only, availability, exploration and fallback;
- normalization: field provenance, defaults, invalid URL and degradation.

Changes to API, MCP, UI, storage, Deep or security also require their existing
integration/security tests. New providers require the complete governance sheet
in [`docs/gobernanza-providers.md`](docs/gobernanza-providers.md) before network
code.

## Local gate

Run the same commands and order as CI:

```bash
cargo fmt --all -- --check
cargo test --workspace
cargo check --workspace --benches
cargo clippy --workspace --all-targets -- -D warnings
cargo audit
cargo deny check
cargo cyclonedx
```

`contract-gate` must be configured as a required check in branch protection.
GitHub currently rejects branch protection for this private user-owned
repository unless its plan is upgraded or the repository becomes public. Until
one of those owner decisions is made, contributors must treat a green
`contract-gate` as mandatory review evidence; this is not equivalent to an
enforced hosting control.

## Licensing

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in AMATL is licensed under either Apache-2.0 or MIT, at the user's
choice, with no additional terms. See `LICENSE-APACHE` and `LICENSE-MIT`.
