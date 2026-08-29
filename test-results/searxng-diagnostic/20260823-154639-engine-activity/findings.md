Generated test artifact — SearXNG engine activity validation — do not treat as project documentation.

# Findings

## Static inspection

The effective SearXNG override enables `duckduckgo`, `mojeek`, and `qwant` (`disabled: false`); it disables `brave`, `google cse`, `startpage`, and `bing`. The installed engine settings place all three evaluated engines in `general`/`web`, so they apply to a general search.

AMATL sends `q`, `format=json`, and `pageno=1`; it does not send `engines` or `categories`. In installed SearXNG code, if no category is specified (including cookie preference), the default selected category is `general`. Thus an ordinary AMATL search leaves SearXNG to select the enabled general engines; configuration alone does not prove execution.

The explicit tests used SearXNG's normal `engines=<name>` request parameter. This transient parameter selected one engine per request; it did not modify configuration.

## Runtime evidence

### DuckDuckGo — CONFIRMED_ATTEMPT_FAILED

The `engines=duckduckgo` response at `2026-08-23T15:46:03-07:00` was HTTP 200 with zero results/answers and public `unresponsive_engines=[['duckduckgo','access denied']]`. The container log one second later records a SearXNG DuckDuckGo POST failure, HTTP 403, and suspended time. This is direct evidence of `SearXNG → DuckDuckGo attempt → access denied`, not merely a configured engine.

### Mojeek — UNKNOWN

The `engines=mojeek` response at `2026-08-23T15:46:23-07:00` was HTTP 200 with zero results/answers, no `unresponsive_engines`, and no public error. The inspected recent logs contain no Mojeek event. An explicit selection shows that the request was submitted to SearXNG, but zero results without an engine attribution does not prove that SearXNG executed Mojeek or received an upstream response. It is neither `CONFIGURED_ONLY` nor `NOT_ACTIVE`; it has runtime context but insufficient attribution.

### Qwant — CONFIRMED_ACTIVE

The `engines=qwant` response at `2026-08-23T15:46:39-07:00` was HTTP 200 with 10 results, zero answers, no unresponsive engines, and no public error. Since Qwant was the sole explicitly selected engine, these results are runtime-attributable to Qwant: `SearXNG → Qwant → results`.

## Baseline interpretation

The earlier Baseline SearXNG v1 observed variable `10/0` results and partial SearXNG responses, but did not expose individual engine identities. This diagnostic demonstrates a concrete failing DuckDuckGo path and a concrete working Qwant path. It supports, but does not prove, that engine availability contributes to observed partiality. It cannot explain the full `10/0` pattern causally: the baseline's zero-result executions did not preserve per-engine attribution, and Mojeek remains unknown.
