Generated preflight artifact — AMATL SearXNG Baseline v2 readiness — do not treat as project documentation.

# AMATL SearXNG Baseline v2 — post-change preflight

- Generated: 2026-08-23T17:32:28-07:00
- Scope: read-only operational precheck. The sole network action was one local, non-search HTTP request to the SearXNG root endpoint.
- No benchmark query, SearXNG search, upstream-engine request, AMATL build, configuration change, service action, or Marginalia request was made.
- Decision: `BLOCKED:SEARXNG_FIXTURE_NOT_RUNNABLE`

The live SearXNG service itself is ready: AMATL resolves its endpoint from `SEARXNG_INSTANCE_URL` to `http://127.0.0.1:8888`; the host-networked `searxng` container is running and listens on `127.0.0.1:8888`; the root returned HTTP 200 in 21.614 ms. The installed SearXNG loader resolved DuckDuckGo, Mojeek, and Qwant as disabled, and comparison with the documented backup found only the authorized three `disabled: false` to `disabled: true` deltas.

The existing isolation TOML is intact, parses, selects only `searxng`, and excludes Marginalia. However, its documented invocation requires `target/debug/amatl`, and no executable AMATL binary exists under `target/`. Building is prohibited by this precheck, so the fixture cannot currently be executed without an out-of-scope build. This prevents authorization to execute the baseline.

See `preflight.json` for structured evidence and `findings.md` for the control-by-control assessment.
