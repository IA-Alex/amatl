# Continuidad de desarrollo — AMATL

## Estado

Las fases 0–9 están implementadas. `plan_amatl.md` y `fase_a_contratos.md`
son los contratos rectores y no deben modificarse durante desarrollo ordinario.

| Fase | Estado | Entrega |
|---|---|---|
| 0–4 | Cerrada | Core Search, providers, pipeline, SQLite y routing |
| 5–7 | Cerrada | Deep, ranking v2 y Gap Analyzer |
| 8 | Cerrada | UI estática y responsiva |
| 9 | Cerrada | API Axum, MCP, TLS y hardening |

## Trazabilidad de la sesión cerrada

- Se preservaron sin edición los documentos rectores: `plan_amatl.md` y
  `fase_a_contratos.md`.
- Se corrigieron los contratos y el esqueleto Rust antes de incorporar red o
  infraestructura; la implementación se completó por fases 0–9.
- Se resolvió un error de compilación causado por derivar `Eq` sobre un tipo que
  contenía `Classification` no-`Eq`; el workspace quedó compilable.
- El alcance quedó deliberadamente acotado: sin LLM obligatorio, crawler
  masivo, cache/infraestructura fuera de contrato ni lógica duplicada en bordes.
- La validación final registrada fue: formato, pruebas de workspace, Clippy,
  Cargo Audit y Cargo Deny. `cargo deny` puede informar duplicados transitivos;
  no son un fallo mientras el comando finalice correctamente.

## Remediación contractual posterior

- Repositorio Git inicializado en `main`; CI `contract-gate` cubre formato,
  tests, benches, Clippy, Audit, Deny y SBOM.
- Classification cubre las 11 categorías, secundarias, confidencias y prioridad
  de señales explícitas; `code` y `academic` ya son alcanzables por routing.
- `FinalUrl` es un newtype exclusivo de Deep; `Rank` y `RankingScore` rechazan
  valores fuera de sus invariantes al construir y deserializar.
- Canonicalization emite `degraded` cuando conserva escapes porcentuales
  malformados; la degradación queda tipada.
- Parallel Search aplica concurrencia global/per-provider, backoff exponencial
  y jitter no determinista configurables.
- CLI devuelve `1` en `failure`, conserva `0` en `partial_success` y usa nombres
  snake_case; logs redirigidos son JSON con `ts`, `level`, `target`, `msg` y
  `context`.
- Políticas de Ranking, Diversity, Ranking v2 y Gaps admiten calibración válida
  sin recompilar. Ranking v2 se compara contra Ranking MVP sobre un corpus
  humano etiquetado: nDCG@3 `0.655768 → 0.919378`.
- Existen property tests, benchmarks Criterion y parsing HTML estructural con
  `scraper` 0.27; Cargo Audit y Cargo Deny finalizan correctamente.

## Arquitectura vigente

`amatl-core` contiene toda la lógica de producto. CLI, API y MCP consumen
`AmatlService`; no duplicar orquestación, Budget ni contratos.

- UI: `crates/amatl-ui/`
- HTTP/MCP: `crates/amatl-server/`
- CLI: `crates/amatl-cli/`
- Servicio compartido: `crates/amatl-core/src/service.rs`

## Invariantes no negociables

- Search nunca ejecuta Deep/fetch/render/extract.
- `SearchOrchestrator` y `DeepOrchestrator` son dueños exclusivos de Budget.
- Search expone `original_url` y `canonical_url`, nunca `final_url`.
- MCP tiene límites más estrictos que CLI.
- API/MCP usan token; bind remoto exige token y TLS.
- UI/API/MCP no contienen lógica de producto duplicada.
- No introducir LLM obligatorio, agent loops, crawler masivo ni infraestructura no aprobada.

## Verificación antes de continuar

```bash
cargo fmt --all -- --check && cargo test --workspace && cargo check --workspace --benches && cargo clippy --workspace --all-targets -- -D warnings && cargo audit --no-fetch && cargo deny check
```

## Ejecución local

```bash
export AMATL_SERVER_TOKEN="$(openssl rand -hex 32)"
cargo run -p amatl-cli -- serve
```

Servidor: `127.0.0.1:8080`; UI `/`, health `/health`, API `/search`, `/deep`,
`/providers`, MCP `/mcp`.

## Siguiente paso

No existe una Fase 10 en el golden template. Antes de ampliar alcance, crear
una decisión explícita basada en `plan_amatl.md`, con contrato, pruebas y
criterios de aceptación; no iniciar componentes por inferencia.

Pendientes externos, no defectos del core: marcar `contract-gate` como requerido
en la protección de rama del hosting, completar aprobaciones/credenciales de
providers y habilitar Renderer sólo cuando exista aislamiento CDP verificable.
