Generated test artifact — SearXNG diagnostic — do not treat as project documentation.

# Alcance y metodología

- Fecha/hora de inicio: 2026-08-23T15:04:42-07:00
- AMATL commit inspeccionado: `48d0a9a24b3365e996a3b5e63eb3792fe70ed57a`
- Evidencia previa, no modificada: `test-results/searxng/20260823-145832/`.
- Alcance: diagnóstico estático y trazabilidad causal de router, adapter, pipeline y canary.
- Tráfico adicional: ninguno. La evidencia previa ya contiene la ejecución mínima de `rust async`; las demás respuestas solicitadas se determinan por código estático.
- Código, configuración existente, variables de entorno, servicios, SQLite y cachés: no modificados.

## Límites de evidencia

El CLI normal conserva el `SearchResponse` agregado, no el cuerpo HTTP de SearXNG ni el `ProviderResult` interno. Por ello el diagnóstico puede demostrar la ruta interna y el conjunto de resultados que llega al pipeline, pero no identificar el contenido o la causa upstream concreta de `results`/`answers` vacíos sin una superficie AMATL adicional que exponga datos no sensibles.
