# Auditoría AMATL — re-análisis con los cambios nuevos (HEAD `4d4c60b6`)

Continúa `audit/auditoria-2026-11-13.md`. Alcance: **analizar la herramienta con sus
nuevos cambios**, es decir, validar la remediación de los commits `45559cb2`
(erradicación de providers retirados) y `4d4c60b6` (SSRF, higiene, advisories), y
determinar qué hallazgos siguen abiertos.

- Fuente: `/home/panda/amatl`, árbol de trabajo **limpio**.
- HEAD al analizar: `4d4c60b6` (`fix(core,server,ui,deps): close the SSRF, hygiene and advisory findings`).
- Modo: estático + ejecución real de gates. **No se modificó** ningún fichero de producto.
  Este informe se escribe en `audit/` por convención del repo.

---

## 1. Veredicto

La remediación es **sólida y verificada**: el bypass de SSRF está cerrado con test de
regresión, `fmt`/`clippy`/`doc`/`test` vuelven a verde, y la cadena de suministro pasa de
**5 vulnerabilidades + 1 unsound a 1 sola** (sin fix compatible, honestamente documentada).
Quedan **dos incoherencias código↔documentación** ya conocidas y **una semántica de
`/ready`** sin resolver.

| Severidad | Abiertos | Cerrados por los cambios nuevos |
|---|---|---|
| Crítico | 0 | — |
| Alto | 1 | 2 (SEC-001, CHAIN-001 parcial) |
| Medio | 2 | 1 (DOC-001) |
| Bajo | 4 | 3 (BUILD-001, FE-001, FE-002, BE-001) |

---

## 2. Gate local ejecutado hoy (verificación real)

| Comando | Resultado |
|---|---|
| `cargo fmt --all -- --check` | **OK** (antes rojo) |
| `cargo check --workspace --benches --locked` | OK |
| `cargo test --workspace --locked` | **698 passed, 0 failed, 2 ignored** (antes 695; +3 tests) |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` | OK |
| `RUSTDOCFLAGS=-D warnings cargo doc --workspace --no-deps --locked` | OK |
| `cargo run -p amatl-cli -- benchmark ranking-v2 --json` | `passed=true` (ndcg Δ +0.2636) |
| `cargo audit` / `cargo deny check` | **ROJO: 1 vuln** (`libsqlite3-sys`, ver §3) |

---

## 3. Remediación verificada (commits nuevos)

### SEC-001 (Alto) — SSRF por IPv6 IPv4-compatible: **CERRADO**
`crates/amatl-core/src/security.rs` sustituye `to_ipv4_mapped()` por `to_ipv4()`, que cubre
**tanto** la forma *mapped* (`::ffff:a.b.c.d`) como la *compatible* (`::a.b.c.d`, RFC 4291
§2.5.5.1), y añade el rechazo de NAT64 `64:ff9b::/96` y 6to4 `2002::/16`. Tests de regresión:
`rejects_ipv4_compatible_ipv6_that_embeds_private_or_metadata_space` y
`rejects_ipv4_embedding_transition_ranges` — **7/7 verdes**. Esto era el riesgo Alto #1 del
informe previo y está genuinamente resuelto, no silenciado.

### CHAIN-001 (Alto) — advisories: **PARCIALMENTE CERRADO**
`rmcp 3.10.0`, `rustix 1.1.6`, `quinn-udp 0.5.16` (fija RUSTSEC-2026-0457/-0461/-0462, -0486,
-0538). Queda **una**:

```
libsqlite3-sys 0.30.1  RUSTSEC-2026-0459 (6.8 medium, CVE-2026-119375)
  path: sqlx 0.8.6 -> sqlx-sqlite -> libsqlite3-sys
```

No hay fix compatible: `sqlx 0.8.6` topa `^0.30` y `sqlx 0.9.0` topa `<0.38.0`, mientras el
fix es `>=0.39.0`. Está **documentado** en `docs/security/supply-chain.md` y **sin** `ignore`
en `deny.toml` (la política prohíbe el allowlist silencioso). El test offline
`tests/supply_chain_contract.rs` mantiene floors para los advisories remediables y **excluye
explícitamente** este caso con justificación escrita — correcto.

> **Matiz de threat-model** que conviene añadir a `supply-chain.md`: el advisory dice que
> "only a process that opens a database file, with its `-wal`, that a less trusted party
> could have written is exposed". En AMATL el SQLite es almacenamiento local que el propio
> usuario controla ⇒ **explotabilidad baja** (vector local, sin superficie remota). Conviene
> decirlo explícitamente para que el rojo de CI se lea como deuda acotada y no como riesgo
> abierto de producción.

### BUILD-001 / FE-001 / FE-002 / BE-001 / DOC-001 — **CERRADOS**
- `cargo fmt --check` OK; CHANGELOG ya no está corrupto; árbol limpio.
- `app.js` envuelve `decodeURIComponent` en `try/catch` (`?q=%` ya no aborta el bootstrap).
- `app.js` mapea `provider.status` por allowlist (`PROVIDER_STATUS_CLASSES`).
- `robots.rs` evicta **una** entrada (la más antigua) en vez de vaciar la caché, con test.
- `supply-chain.md` ya declara que `cargo deny check` está rojo por este advisory.

### Refactor de providers (`45559cb2`) — **LIMPIO**
Purga total de DuckDuckGo/Brave: 0 residuos en `crates/*/src` (las coincidencias `brave*`
restantes son dominios de tracker ajenos). `RETIRED_PROVIDERS`/`is_retired_provider`
eliminados; solo quedan adaptadores SearXNG y Marginalia. El BREAKING (config con fuente
retirada ahora falla con `unknown enabled provider`) está cubierto por tests.

---

## 4. Hallazgos abiertos (verificados vigentes)

### [ALTA] `GET /deep` contradice el contrato "POST-only"
**Evidencia**: `crates/amatl-server/src/routes.rs:68` registra `get(deep)`; el handler
`deep` en `crates/amatl-server/src/lib.rs:995` responde un `DeepResponse` completo
(incluye `final_url`). **Y `crates/amatl-server/openapi.yaml:121` documenta la operación
`deepGet` como soportada.** Sin embargo `docs/testing.md:35`, `crates/amatl-ui/src/lib.rs:297`
y el `CHANGELOG` afirman que Deep es POST-only. El único test que toca esa ruta
(`tests.rs:1540`) **autoriza** `GET /deep?q=test` y solo comprueba el header `X-Request-Id`.
**Por qué importa**: es una ruta de recuperación de red con `final_url` documentada en el
contrato pero excluida del relato de seguridad; ningún test defiende la invariante POST-only.
**Cierre**: decidir una de las dos — (a) retirar `get(deep)` del router y de `openapi.yaml`
y añadir test que exija `405`/`404` en `GET /deep`, o (b) aceptar GET como contrato público
y corregir `docs/testing.md`, `amatl-ui/src/lib.rs` y el CHANGELOG. Hoy conviven las tres
versiones.

### [MEDIA] `docs/api/mcp.md` documenta 5 tools; el código expone 6
**Evidencia**: `crates/amatl-core/src/config/server.rs:555` → `MCP_TOOLS: [&str; 6]` =
`search, deep_search, fetch, providers, status, answer`. `docs/api/mcp.md` lista solo 5 en su
tabla (líneas 20–24) y afirma en la línea 90 "list exactly these **five** tools". La tool
`answer` (`mcp.rs:189`) está autorizada (`authorize(&context, "answer")`) pero **no aparece**
en la documentación.
**Por qué importa**: un cliente MCP que lea la doc no sabrá que `answer` existe ni su scope;
la memoria `server/http-mcp` también dice "cinco".
**Cierre**: añadir la fila de `answer` (contrato `{ "query": string }` → respuesta grounded,
requiere backend de completion) y cambiar "five" por "six"; actualizar la memoria.

### [MEDIA] `/ready` — `sources_available` no equivale a fuentes utilizables
**Evidencia**: `crates/amatl-server/src/lib.rs:2771` cuenta `ProviderSurfaceStatus::Available`
**sin** excluir circuitos abiertos, mientras la decisión de readiness (`status.status`)
**sí** los excluye. Puede darse `HTTP 503`, `status=degraded`, `sources_available=1`.
Ya señalado como **H2** en `amatl_revision_critica.md` y **sigue abierto**.
**Por qué importa**: automatización que sondee `sources_available` puede leer "1 disponible"
en un servicio que se declara `degraded`/no listo.
**Cierre**: o renombrar el campo a `sources_advertised` (semántica `available`), o añadir
`sources_usable` versionado; documentar la diferencia sin cambiar el significado actual.

### [BAJO] Deuda menor que permanece
- CLI con código de salida único (`1`) sin clases de error.
- `start.sh` hace `source .env` en el shell del proceso.
- Sin unit tests propios en `providers.rs` (fachada), `model.rs` y ~10 `config/*.rs`.
- `browser_e2e` es fail-closed **solo tras** `AMATL_BROWSER_E2E=1` (documentado; sin opt-in
  los tests retornan `ok`, así que un checkout sin driver no los marca como *ignored*).

---

## 5. Lo descartado sin hallazgo

- Gate completo de formato/test/clippy/rustdoc: **verde**.
- Determinismo y aislamiento de la suite: 698/0 en la corrida completa (sin el flake
  TEST-001 del informe previo, que no reapareció).
- Purga de providers retirados: completa y sin residuos en código de producto.
- `deny.toml`: sin allowlist silencioso; `wildcards = "deny"`, registries fijados.

---

## 6. Roadmap de cierre (ordenado por dependencia e impacto)

| # | Tarea | Sev. | Esf. | Dependencias |
|---|---|---|---|---|
| 1 | Decidir y unificar `GET /deep` (router + `openapi.yaml` + docs + test de invariante) | Alta | S | — |
| 2 | Documentar la tool `answer` y corregir "five→six" en `docs/api/mcp.md` + memoria | Media | XS | — |
| 3 | Resolver semántica `sources_available` vs readiness (renombrar o añadir campo) | Media | S | — |
| 4 | Añadir el matiz de threat-model de RUSTSEC-2026-0459 a `supply-chain.md` | Baja | XS | — |
| 5 | Plan de salida de `libsqlite3-sys` (ADR de waiver firmado o espera a `sqlx` con `libsqlite3-sys >=0.39`) | Media | M | gobernanza |

Los ítems 1–4 son cambios locales y verificables sin red; el 5 exige decisión de gobernanza
(ADR) y no debe resolverse con `ignore` en `deny.toml` según la política vigente.

---

## 7. Cierre de los ítems 1–3 (2026-11-13)

Aplicado según `audit/plan-remediacion-2026-11-13.md`. Gates en verde (699/0/2,
clippy/fmt/rustdoc OK).

| # | Hallazgo | Cierre |
|---|---|---|
| 1 | `GET /deep` contradice "POST-only" | **CERRADO (opción a)**: `get(deep)` fuera del router, handler `deep` eliminado, operación `deepGet` fuera de `openapi.yaml`, test `deep_is_post_only_and_get_is_method_not_allowed` (405 en GET) y `docs/operacion.md` alineado (`POST /deep`). |
| 2 | `docs/api/mcp.md` documenta 5 tools | **CERRADO**: fila `answer` añadida, "five"→"six", memoria `server/http-mcp` actualizada. El código ya exponía 6 y el test ya las afirmaba. |
| 3 | `/ready` — `sources_available` ≠ utilizables | **CERRADO (aditivo)**: campo `sources_usable` (superficie `available` **y** circuito `!= open`); `sources_available` intacto (sigue igual al gauge). Documentado en `ready()` y `docs/operacion.md`; test del circuito abierto extendido (`available=1`, `usable=0`) y test de forma del cuerpo actualizado. |

Ítems 4 y 5 siguen abiertos y fuera del alcance acordado.
