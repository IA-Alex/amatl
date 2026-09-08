Generated preflight artifact — AMATL SearXNG Baseline v2 readiness — do not treat as project documentation.

# Findings

| Control | Result |
| --- | --- |
| SearXNG configured in AMATL | PASS — enabled in `amatl.toml`; approved `searxng-v1` provider |
| Effective endpoint | PASS — `http://127.0.0.1:8888`, sourced at runtime from `SEARXNG_INSTANCE_URL` |
| Deployment | PASS — running Docker container `searxng` (`searxng/searxng:latest`), host network |
| Service running | PASS — running; container PID 235350; no healthcheck defined |
| Listener present | PASS — `127.0.0.1:8888` TCP LISTEN |
| Endpoint/listener match | `ENDPOINT_MATCH` — HTTP, 127.0.0.1, port 8888 and root base path are coherent |
| Minimal reachability | PASS — one non-search `GET /`: HTTP 200, 21.614 ms |
| DuckDuckGo disabled | PASS — effective loader: `true` |
| Mojeek disabled | PASS — effective loader: `true` |
| Qwant disabled | PASS — effective loader: `true` |
| Configuration drift | `NO_DRIFT_OBSERVED` — live file vs documented backup has only the three authorized false-to-true deltas |
| SearXNG-only fixture | BLOCKED — TOML is valid and isolates SearXNG, but its required `target/debug/amatl` runner is absent and building is out of scope |
| Baseline v1 dataset | PASS — exact `dataset.json` parses, contains Q01–Q10; v1 documents 3 sequential rounds, 3 s interval and recoverable taxonomy |
| Marginalia isolated | PASS — fixture has `enabled = ["searxng"]`; no Marginalia section or request |

## AMATL configuration and effective value

`amatl.toml` configures SearXNG in `providers.enabled`, declares `SEARXNG_INSTANCE_URL` as its credential/environment source, and sets the provider/global timeouts to 20,000/45,000 ms. It does not contain an endpoint literal. AMATL's factory parses that environment value when present and otherwise defaults to `http://127.0.0.1:8888`. The environment value was present and resolved to `http://127.0.0.1:8888`; this is the effective runtime endpoint.

## Running instance and effective SearXNG settings

The running `searxng` container uses host networking and mounts the `searxng-data` volume at `/etc/searxng`. `SEARXNG_SETTINGS_PATH` is unset in that instance. The instance's own venv loader resolved 345 engines and reported the requested entries as `duckduckgo=true`, `mojeek=true`, and `qwant=true` for `disabled`. This proves the active `/etc/searxng/settings.yml` overlay plus installed defaults, rather than a historical file, is in use.

The documented 20260823-161754 change names a backup in that exact mounted volume. A live, read-only diff from that backup to the current file emitted exactly three value changes: the documented disabled false-to-true changes. No later configuration drift is observed.

## Fixture and baseline gate

The isolation fixture exists, parses as TOML, selects only SearXNG, excludes Marginalia, and disables persistence/provider cache. It needs no source change. Its recorded interface is `target/debug/amatl --config-file ...`; there is no executable AMATL binary in `target/`. Running it would require an out-of-scope build, so the fixture cannot be used for the baseline in the present environment.

The exact v1 dataset is available and structurally intact. The v1 README supplies the historical execution contract: Q01–Q10, three repetitions, 3-second interval, and the `PARTIAL_SUCCESS` / `SUCCESS` / `ZERO_RESULTS` / `FAILURE` taxonomy.

## Prior transport failure

The 20260823-165200 artifact records a generic AMATL transport error and no SearXNG probe event, but no lower-level socket, DNS, HTTP, or container-state cause. Current availability is a distinct observation. It does not prove that the earlier failure was caused by the current configuration or service state, nor does it disprove a transient cause. Classification: `UNKNOWN`.

## Decision

`BLOCKED:SEARXNG_FIXTURE_NOT_RUNNABLE`
