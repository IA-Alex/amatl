Generated test artifact — SearXNG characterization — do not treat as project documentation.

# Alcance y metodología

- Inicio: 2026-08-23T14:58:32-07:00
- AMATL commit: `48d0a9a24b3365e996a3b5e63eb3792fe70ed57a`
- Provider evaluado: `searxng` (`searxng-v1`)
- Interfaz: `target/debug/amatl --config amatl-isolated.toml provider-canary searxng <consulta> --json`
- Instancia: resuelta por AMATL a través de `SEARXNG_INSTANCE_URL`; su valor no se registra.
- Aislamiento: configuración nueva dentro de este directorio; `persistence.enabled=false`, `history_enabled=false`, ambas cachés deshabilitadas, telemetría persistente deshabilitada; una sola consulta a la vez; sin reintentos.
- Tiempo de proveedor: 20 000 ms. Tiempo global: 45 000 ms.
- Consultas previstas: 3.

## Batería

1. `rust async` — consulta general de referencia.
2. `tokio site:docs.rs` — filtro de dominio aproximado por el adapter.
3. `rust lang:es region:MX after:2025-01-01 filetype:pdf` — filtros que el adapter declara sin soporte nativo; permite observar el contrato publicado por AMATL.

Las salidas JSON de AMATL y stderr de cada ejecución se conservarán sin transformación en archivos por consulta. `results.jsonl` recopilará evidencia estructurada derivada de esas salidas, y `metrics.json` contendrá agregados deterministas.
