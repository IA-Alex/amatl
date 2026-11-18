# Plan de remediación — cierre de hallazgos abiertos (tareas 1–3)

Fuente: `audit/auditoria-2026-11-13-post-remediacion.md` §6, ítems **1, 2, 3**.
Alcance acordado con el usuario. Edición simbólica vía Serena; gates del
fullstack-dev-kit antes de cerrar.

## Tarea 1 — Unificar `GET /deep` (Alta) → **retirar GET**

Contrato decidido: **POST-only**. Se alinea con `docs/testing.md`, `amatl-ui/src/lib.rs`
y el CHANGELOG, que ya lo declaran así; la ruta `GET` era el residuo.

Cambios:
1. `crates/amatl-server/src/routes.rs:68` → `"/deep" => [post(deep_post)]`.
2. `crates/amatl-server/src/lib.rs` → eliminar el handler `deep` (queda huérfano y
   `clippy -D warnings` lo marcaría como dead code). Mantener `deep_post`.
3. `crates/amatl-server/openapi.yaml` → eliminar la operación `get:`/`deepGet`
   (líneas 121–140). El guard bidireccional
   `openapi_operations_match_router_inventory` exige que contract == router.
4. `crates/amatl-server/src/tests.rs`:
   - `request_id_header_is_present_on_all_api_responses`: quitar `("GET","/deep?q=test")`
     del array.
   - Añadir test **de invariante** `deep_is_post_only_and_get_is_method_not_allowed`:
     `GET /deep?q=test` autenticado → `405 Method Not Allowed`; `POST /deep` sigue
     funcionando.

Verificación: el guard de cobertura OpenAPI queda verde sin la operación GET.

## Tarea 2 — Documentar la tool `answer` (Media)

Solo documentación; el código ya expone 6 tools (`MCP_TOOLS: [&str; 6]`) y el test
`...tools/list` ya afirma las 6.

Cambios en `docs/api/mcp.md`:
1. Añadir fila `answer` a la tabla de tools: input `{ "query": string }` → respuesta
   grounded con citas (`Answer`); requiere backend de completion configurado por el
   operador; scope `answer` (reutiliza el de Deep).
2. Línea ~90: "list exactly these **five** tools" → "**six**".
3. Actualizar la memoria `server/http-mcp` ("cinco" → "seis" + tool `answer`).

## Tarea 3 — `sources_available` vs readiness (Media)

El contador y su divergencia son **deliberados** y están pinneados por dos tests
(`ready_reports_degraded_..._open_circuit`, `source_gauges_count_declared...`).
`SourceStatus` ya expone `circuit: CircuitState`, así que la divergencia es
determinista y comprobable.

Cierre **aditivo** (no rompe el contrato con el gauge `amatl_sources_available`):
1. `crates/amatl-server/src/lib.rs` `ready()`: añadir campo `sources_usable` =
   nº de fuentes `Available` **con circuito `!= Open`** (usable ahora mismo), junto a
   `sources_available` (semántica de superficie, sin cambios).
2. Actualizar el doc comment de `ready()` y `docs/operacion.md` para explicar la
   diferencia `available` (superficie) vs `usable` (circuito cerrado/half-open).
3. Tests: extender el test del circuito abierto para afirmar
   `sources_available == 1` **y** `sources_usable == 0` tras abrir el circuito.

## Gates (fullstack-dev-kit)

| Gate | Comando |
|---|---|
| Formato | `cargo fmt --all -- --check` |
| Cobertura de lo tocado | tests nuevos de #1 y #3 |
| Clippy | `cargo clippy --all-targets --all-features --locked -- -D warnings` |
| Tests | `cargo test --workspace --locked` |
| Seguridad (stack Rust) | revisión SSRF/scope: `GET /deep` era una superficie de recuperación de red; su retirada **reduce** superficie |
| Rustdoc | `RUSTDOCFLAGS=-D warnings cargo doc --workspace --no-deps --locked` |
| pr-review | revisión del diff antes de cerrar |

`cargo audit` seguirá rojo por `libsqlite3-sys` (tarea 5, fuera de alcance).

## Estado

**APLICADO** (2026-11-13). Gates en verde tras el cambio:

| Gate | Resultado |
|---|---|
| `cargo fmt --all -- --check` | OK |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` | OK |
| `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --locked` | OK |
| `cargo test --workspace --locked` | **699 passed, 0 failed, 2 ignored** |
| Guard bidireccional OpenAPI (`openapi_operations_match_router_inventory`) | OK (contract == router sin `deepGet`) |
| Test de invariante `deep_is_post_only_and_get_is_method_not_allowed` | OK (405 en GET) |
| Test `ready_reports_degraded_...` extendido | OK (`available=1`, `usable=0`) |

`cargo audit` sigue ROJO por `libsqlite3-sys` (tarea 5, fuera de alcance, ya documentado).

## Fuera de alcance

- Tarea 4 (matiz threat-model RUSTSEC-2026-0459) y tarea 5 (ADR de waiver).

**Actualización 2026-11-15:** ambas se trabajaron en un ciclo aparte. La 4 está cerrada
(«Threat-model note» en `docs/security/supply-chain.md`); la 5 está aplicada sólo en su
parte ejecutable (pin `sqlx = "=0.8.6"` + guarda `sqlx_is_pinned_to_the_audited_line`),
sin waiver: la firma de `ADR-017` y el gate rojo de `cargo audit` siguen pendientes. Detalle
y evidencia en §8 de `audit/auditoria-2026-11-13-post-remediacion.md`.
- Cualquier reescritura de la historia: si un test contradice el cambio, se para y se
  pregunta.
