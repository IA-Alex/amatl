# Campaign entrypoint validation

Offline/static validation of `--campaign`. The campaign branch was never invoked
with a valid AMATL binary or fixture, so no AMATL process, provider request,
HTTP request, DNS request, SearXNG request, or Marginalia request occurred.

The historical `benchmarks/searxng-v2/20260823-190535` directory was read only
as a frozen dataset source. Its benchmark output was neither used as an
implementation reference nor modified.
