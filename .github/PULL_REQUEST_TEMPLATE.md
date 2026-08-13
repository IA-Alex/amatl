## Summary

Describe the outcome and why it is needed.

## Contract impact

- [ ] `plan_amatl.md` and `fase_a_contratos.md` are unchanged, or this is an explicitly approved dedicated contract proposal.
- [ ] Rust, JSON, SQLite, CLI, logs, API and MCP names remain consistent.
- [ ] `schema_version`, SemVer, adapter/extractor versions and migrations are treated independently.
- [ ] Product logic remains in `amatl-core`; surfaces do not duplicate it.
- [ ] Budget ownership and Search/Deep boundaries remain intact.

## Verification

- [ ] Valid and degraded input are covered.
- [ ] Typed errors and partial results are covered where applicable.
- [ ] Invariants and exhausted Budget are covered.
- [ ] Required provider/canonicalization/deduplication/Budget/ranking/Fetcher/extractor/router/normalization contract tests were added or remain applicable.
- [ ] Security, property, integration and benchmark coverage was updated when affected.
- [ ] Documentation/configuration/changelog was updated.
- [ ] No secrets, credentials, local databases or `amatl.toml` were committed.
- [ ] I ran the complete `contract-gate` command sequence from `CONTRIBUTING.md`.

## Evidence and residual risk

List tests, benchmark output and any known limitation. Do not include secrets.
