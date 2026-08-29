Generated test artifact — SearXNG effective engine audit — do not treat as project documentation.

# Findings

111 is a configuration count, not an execution count. SearXNG's installed request path defaults this AMATL request to `general`; static module categories yield 18 candidates. `wikidata` is then demonstrably unavailable for selection because its INIT failed and its processor was not registered. The remaining 17 cannot be attributed from existing logs or the JSON surface.

The post-change normal AMATL observation proves SearXNG responded HTTP 200 with zero results and no exposed unresponsive engines. It proves neither an upstream attempt nor a contributor for any individual remaining engine. No controlled query was needed or issued.

AMATL maps an HTTP-200 SearXNG response with empty `results`/`answers` and an empty `unresponsive_engines` array to `ProviderExecutionStatus::Success`. The orchestration marks final `success` when results are empty only if there is no failed/partial provider, degradation, or no-provider condition. Thus `success + 0 results` is permitted by the present contract. Marginalia's rate limit can make the global execution non-ideal but did not make SearXNG partial and does not identify a SearXNG engine.

This is not a contradiction of Baseline SearXNG v1: the baseline's zero-result case had SearXNG partial (`unresponsive_engines` nonempty), which correctly led to AMATL `failure/no_usable_results`; the later observation has no exposed unresponsive engine and therefore follows the other permitted branch.

Remaining decision-critical unknown: which of the registered general candidates SearXNG actually scheduled and what each returned during the normal post-change request. Existing observability does not reveal it.
