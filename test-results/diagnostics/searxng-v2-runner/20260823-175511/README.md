# Auditoría forense del runner Baseline SearXNG v2

Generado: 2026-08-23T17:55:11-07:00.

Alcance: inspección estática de artefactos históricos y código fuente, sin ejecutar
AMATL, la fixture ni búsquedas, y sin contactar proveedores.

Fuentes primarias: `../../benchmarks/searxng-v2/20260823-174253/`,
`../../benchmarks/searxng-v1/20260823-153023/`, los dos preflights v2 y la
fixture `../../searxng/20260823-145832/amatl-isolated.toml`.

Conclusión: `RUNNER_ROOT_CAUSE_UNKNOWN`. Los registros prueban un plan efectivo
de 47 invocaciones y el patrón exacto de 17 duplicados, pero no se conservó un
runner, script, historial de shell, log de proceso ni comando que permita atribuir
la generación de ese plan a un bloque de código concreto. No se modificó ningún
artefacto histórico.

Véanse `reconstruction.json` para la secuencia normalizada y `findings.md` para
la cadena de evidencia y la propuesta de validación offline.
