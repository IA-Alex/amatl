# amatl-core

Núcleo compartido por todas las superficies de AMATL (CLI, UI, API y MCP).
Contiene la lógica de producto: búsqueda, orquestación de proveedores,
persistencia, seguridad, caché y telemetría. Mantén la lógica de producto
fuera de los crates de superficie (`amatl-cli`, `amatl-server`, etc.).

## Módulos públicos

| Módulo | Responsabilidad |
|--------|-----------------|
| `audit` | Auditoría de eventos de seguridad y retención. |
| `budget` | Presupuestos de búsqueda y profundización (tiempo, bytes, llamadas). |
| `cache` | Caché de resultados por proveedor (TTL, LRU, cuota de bytes). |
| `canonical` | Canonicalización de URLs. |
| `circuit` | Circuit breakers por proveedor. |
| `classify` | Clasificación de consultas. |
| `config` | Configuración tipada (proveedores, egress, inferencia, renderizado). |
| `dedupe` | Deduplicación de resultados. |
| `deep` | Orquestación de búsqueda profunda (deep search). |
| `diversity` | Políticas de diversidad de resultados. |
| `document_cache` | Caché de documentos extraídos versionada. |
| `errors` | Catálogo de códigos de error. |
| `evidence` | Análisis de evidencia y fragmentos. |
| `execution` | Orquestador de búsqueda paralela (`SearchOrchestrator`). |
| `extract` | Extracción de contenido (HTML nativo, Trafilatura). |
| `fetch` | Fetch seguro con resolución DNS y política de egress. |
| `gaps` | Análisis de huecos y subconsultas. |
| `inference` | Backends de embeddings y reranking. |
| `ingest` | Ingesta de documentos. |
| `model` | Tipos de dominio (query, resultados, planes de búsqueda). |
| `normalize` | Normalización de resultados. |
| `operational` | Métricas operativas. |
| `planning` | Construcción de planes de búsqueda por proveedor. |
| `progressive` | Resultados progresivos. |
| `providers` | Adaptadores de proveedores (Brave, Marginalia, Mojeek, SearXNG) y registro. |
| `query` | Parsing de consultas. |
| `ranking` / `ranking_v2` | Ranking de resultados. |
| `render` | Renderizado de respuestas. |
| `robots` | Política `robots.txt`. |
| `router` | Enrutamiento adaptativo entre proveedores. |
| `security` | Validación de URLs/SSRF y auditoría de rechazos. |
| `service` | `AmlatService`: lógica de negocio central y superficie pública. |
| `storage` | Persistencia SQLite (`SqliteStorage`). |
| `telemetry` | Telemetría en memoria. |

## Dependencias

Dependencias principales declaradas en el workspace (`Cargo.toml`):

| Crate | Versión | Uso |
|-------|---------|-----|
| `rmcp` | `3.1.2` | Protocolo MCP (server, transporte streamable HTTP). |
| `sqlx` | `0.8` | Acceso a SQLite (runtime Tokio, feature `sqlite`). |
| `tokio` | workspace | Runtime asíncrono. |
| `tracing` | workspace | Logging estructurado. |
| `serde` / `serde_json` | workspace | Serialización. |
| `url` | workspace | Parsing y validación de URLs. |
| `thiserror` | workspace | Definición de errores. |

### Versiones compatibles de `rmcp` y `sqlx`

- **`rmcp`**: se requiere `>= 3.1.2`. La API de servidor y el transporte
  `transport-streamable-http-server` se consideran estables a partir de esa
  versión. No se garantiza compatibilidad con `3.0.x` ni con `4.x` sin
  revisión de la API.
- **`sqlx`**: se requiere `>= 0.8, < 0.9`. Se usa la feature `sqlite` con
  `runtime-tokio` y `default-features = false`. La migración a `0.9` exige
  revisar los tipos de `SqlitePool` y las macros de consulta.

## Cómo contribuir

1. Mantén la lógica de producto en `amatl-core`; las superficies solo orquestan.
2. Sigue el estilo existente: `cargo fmt` y `cargo clippy --workspace -- -D warnings`.
3. Añade tests junto al código (unitarios en `#[cfg(test)]`, de integración en
   `crates/amatl-core/tests/`).
4. Ejecuta la suite completa antes de abrir un PR:
   ```sh
   cargo test --workspace
   ```
5. Documenta cualquier cambio de API en este README y en `docs/`.

## Tests

```sh
# Solo el núcleo
cargo test -p amatl-core --lib

# Todo el workspace
cargo test --workspace
```
