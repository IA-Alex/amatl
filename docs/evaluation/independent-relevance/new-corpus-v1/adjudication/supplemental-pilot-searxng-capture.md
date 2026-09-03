# Supplemental pilot: isolated SearXNG capture

The only capture entry point is `tools/supplemental_pilot_capture.py`.  Its
only contract is `supplemental-pilot-searxng-capture-config.json`; it does not
read `amatl.toml`, construct AMATL providers, or use the multi-provider router.
The effective provider is consequently `searxng`, with no fallback.

Before an authorized capture, verify the frozen inputs without network access:

```sh
python3 tools/validate_supplemental_pilot_preflight.py
python3 tools/test_supplemental_pilot_capture.py
```

The configuration names the frozen universe and its SHA-256.  The transport
refuses an ID/hash mismatch, a non-SearXNG provider, a fallback, dynamic
selection/expansion, a non-frozen query, or an incompatible evidence schema.
It uses at most two attempts for the same immutable query and permitted rank
range.  Every attempt, including empty and failed ones, is retained.

Before separately authorized execution, record the exact operator-approved
SearXNG endpoint in `searxng_endpoint_binding.endpoint` in the isolated
capture configuration.  Its `source` must remain `EXPLICIT_OPERATOR_INPUT`.
The command deliberately has no endpoint flag and never reads AMATL global
configuration.  Request exactly the desired frozen IDs:

```sh
python3 tools/supplemental_pilot_capture.py \
  --query-id sup-s1-001 \
  --allow-network --output /secure/path/raw-evidence.json
```

The versioned raw-evidence protocol is
`amatl.relevance.supplemental-pilot-raw-evidence.v1`.  It includes immutable
universe provenance, query/stratum, SearXNG status, attempt metadata, allowed
ranks and audit fields for only permitted results.  Output is deterministic
JSON (sorted keys); `artifact_sha256` is written at capture time.  Do not put
tokens, cookies, or credentials in the output or endpoint.

Validate and hand the evidence to the existing offline authority; it alone
does canonicalization, deduplication, overlap decisions, and quotas:

```sh
python3 tools/run_supplemental_pilot.py \
  --raw-evidence /secure/path/raw-evidence.json \
  --output /secure/path/offline-runner-attempts.json
```

No network is possible unless `--allow-network` and an endpoint are both
provided.  Failure is closed: correct the reported contract error, regenerate
the evidence, and never substitute another provider or expand a query.
