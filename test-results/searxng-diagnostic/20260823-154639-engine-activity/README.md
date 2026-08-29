Generated test artifact — SearXNG engine activity validation — do not treat as project documentation.

# SearXNG engine activity validation

Read-only diagnostic of the project-controlled SearXNG instance. It distinguishes static configuration, request selection, runtime attempt, and response/error.

## Scope and controls

- Query: `rust async`.
- Three sequential requests, one explicit engine per request: `duckduckgo`, `mojeek`, then `qwant`.
- No retries; 5-second pauses between requests; no upstream sites were called directly.
- The response bodies were processed only to record the permitted counts and public `unresponsive_engines` metadata. No bodies, result URLs, headers, cookies, tokens, or credentials were stored.

## Outcome

| Engine | Classification |
| --- | --- |
| DuckDuckGo | `CONFIRMED_ATTEMPT_FAILED` |
| Mojeek | `UNKNOWN` |
| Qwant | `CONFIRMED_ACTIVE` |

See `engine-activity.json` for the consolidated result and `evidence.jsonl` for individual evidence records.
