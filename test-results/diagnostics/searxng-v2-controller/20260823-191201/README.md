# Diagnóstico estático de controlador y persistencia

Generated static controller/persistence diagnostic — no AMATL or provider execution performed.

Alcance: inspección local de `tools/benchmark_plan_runner.py` (SHA-256 `28e4b64c68533b65c55ab180a063254387f7b99d7985887a2a899f3152ae02c7`) y de `test-results/benchmarks/searxng-v2/20260823-190535/`. La campaña histórica no fue modificada.

Resultado principal: el estado observable de esa ruta contradice el estado conocido indicado en la solicitud. Contiene 30 registros persistidos y declara una campaña completa. Tampoco existe en el árbol local el texto `BASELINE_V2_INVALID:CONTROLLER_TERMINATED_AFTER_9_AMATL_INVOCATIONS`.

Consulte `controller-analysis.md`, `persistence-analysis.md` y `evidence.json`.
