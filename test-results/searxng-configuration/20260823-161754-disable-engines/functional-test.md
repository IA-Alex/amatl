Generated configuration artifact — SearXNG engine isolation — do not treat as project documentation.

# One normal AMATL query

Executed exactly once, after the SearXNG-only restart:

```text
amatl search --json "rust async"
```

| Field | Observed value |
| --- | --- |
| Provider selected / used | `searxng` |
| SearXNG process state | `running` |
| SearXNG result count surfaced by AMATL | `0` |
| AMATL final status | `success` |
| Other provider | `marginalia` failed with its pre-existing public rate-limit response; not changed or retried |
| SearXNG partial providers | none surfaced by AMATL |

The SearXNG log interval from the post-change restart through this one request contains no `duckduckgo`, `mojeek`, or `qwant` execution, result-source, or `unresponsive_engine` entry. The effective loader also excludes all three from enabled engines. Together this is evidence that none participated in the normal AMATL request.

The zero results do not trigger a reversal: general-capable engines remain effective, and the requested change intentionally removes the three target engines.
