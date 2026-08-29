Generated preflight artifact — AMATL SearXNG v2 fixture build readiness — do not treat as project documentation.

# AMATL SearXNG v2 fixture build readiness

- Generated: 2026-08-23T17:36:30-07:00
- Prior blocking artifact reviewed: `test-results/preflight/searxng-v2/20260823-173228/`.
- Prior blocker: `BLOCKED:SEARXNG_FIXTURE_NOT_RUNNABLE`, caused solely by the absence of `target/debug/amatl`.
- Commit: `48d0a9a24b3365e996a3b5e63eb3792fe70ed57a`.
- Minimal build performed: `cargo build -p amatl-cli --bin amatl --locked`.
- Build result: success, dev profile, no tests, benchmark, `cargo run`, AMATL invocation, search, or provider traffic.
- Decision: `READY_FOR_V2`.

`amatl-cli` is the only workspace package declaring the required `amatl` binary. The build generated an executable x86-64 ELF at `target/debug/amatl`. The pre-existing SearXNG-only TOML remains static-valid, resolves that relative runner path, enables only `searxng`, and excludes Marginalia.

No tracked file changed as a result of the build. The three modified tracked documentation files and the existing untracked files/directories were already present before the build and are unchanged. `Cargo.lock` has no diff. Cargo generated only ignored `target/` artifacts.

See `build-readiness.json` for the structured observations.
