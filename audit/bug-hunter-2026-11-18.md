# bug-hunter — Informe consolidado 2026-11-18 (workspace AMATL)

Dos investigaciones en paralelo, solo lectura, con reproducciones ejecutadas fuera
del repo (crate scratch en `/tmp/user/1000/opencode/repro`, sin tocar `amatl.toml`,
`.env` ni SQLite):

- **core + CLI adaptador:** `audit/bug-hunter-core-2026-11-18.md` (B1–B7, con reproducciones T1–T5 ejecutadas)
- **server + UI + CLI:** `audit/bug-hunter-server-ui-cli-2026-11-18.md` (BH-01–BH-04, con sondas contra servidor real mock)

Baseline dinámica propia: `cargo test --workspace --locked` → 25 suites OK, 0 fallos
(`audit/baseline-test-2026-11-18.txt`); `cargo test -p amatl-core` → 406 tests OK.
Los hallazgos **no están cubiertos por tests existentes**.

## Tabla maestra de hallazgos

| ID | Clase | Hallazgo | Evidencia | Gravedad |
|----|-------|----------|-----------|----------|
| B1 | **CONFIRMED_BUG** | Entidades HTML decodificadas tras la validación (`normalize.rs:133-146`) permiten inyectar `ESC`/`NUL`/U+FFFD en títulos/snippets de resultados (inyección ANSI en terminal vía provider hostil) | Reproducción T1 (bytes exactos) + verificación en fuente | **ALTA** |
| B2 | **CONFIRMED_BUG** | Frontier de Deep duplica el mismo documento por variantes de fragmento (`visited` guarda URL con `#`, `child_result` la elimina): 3 fetches donde 1 bastan, `search_result_id` colisionado | Reproducción T2 + verificación en fuente (`deep.rs:440` vs `:663`) | **MEDIA-ALTA** |
| B3 / BH-04 | **CONFIRMED_BUG** | `amatl saved show --id N` barre solo los 200 más recientes → “no existe” falso para ids antiguos; reproduce con 201 guardados (= hallazgo BH-04 del otro frente, unificado) | Reproducción T3 (201 docs, id 1 no encontrado) | MEDIA |
| BH-01 | **CONFIRMED_BUG** | La UI crea clientes MCP con `tools: []` literal (`app.js:1884`): toda llamada MCP responde `scope_denied`; el checkbox «MCP» es un control muerto (solo PATCH por API remedia) | Sonda en vivo con credencial `tools=[]` + verificación de `allows_tool` = `any` sobre la lista | MEDIA |
| BH-02 | **CONFIRMED_BUG** | Clasificación de errores por regex/`includes` sobre el *texto* del mensaje (`app.js:1493,1010,1900-1903`) en vez de `error.status`: 401/403 → “clave inválida” engañosa en el diálogo de secretos de Marginalia, alta de clientes y selector de fuentes | Respuestas reales `{"code":"unauthorized"…}` sin dígitos de estado; verificación en fuente | MEDIA (UX engañosa en diálogo de secretos) |
| BH-03 | **CONFIRMED_BUG** | Paginación sin clamp al total: `?page=10` aceptado por `restoreFromLocation`, eco del servidor sin acotar → «Página 10 de 1» y navegación inerte | Sonda en vivo `page:9 → total_results:1, results:[]` | **BAJA** |
| B4 | **PROBABLE_BUG** | Claves desconocidas/typo en `amatl.toml` ignoradas en silencio (`Config::from_toml` sin `deny_unknown_fields`): `[data_policy] profle = "isolated"` deja **egress abierto** sin aviso — fail-open contra el invariante fail-closed del proyecto | Reproducción T4 ejecutada (`allows_network_egress() = true` con typo) | **ALTA** (seguridad configuracional) |
| B5 | **PROBABLE_BUG** | Redirects seguidos por fetches que **fallan** no se cargan a `DeepBudget.remaining_redirects` (cargo solo en `Ok`, `deep.rs:285-294`): tráfico real hasta `F×R` en vez de `R` | Lectura de código inequívoca; falta reproducción con servidor de redirects | MEDIA |
| B7 | **POTENTIAL_BUG** | `fetch_timeout` de Deep se calcula con `now` anterior a la espera de `Crawl-delay`: Deep puede exceder su deadline hasta 5 s (`MAXIMUM_CRAWL_DELAY_MS`) | Lectura de código; reproducción barata pendiente | BAJA |
| B6 | **POTENTIAL_BUG** | Cache TTL: casts `u64 as i64` sin cota ni `checked_add` (`storage.rs:1332-1334`); TTL gigantesco → nunca fresca. **No alcanzable vía TOML** (i64 límite del parser), solo vía API Rust de la librería | Reproducción T5 con `u64::MAX`; mitigación TOML verificada | BAJA (API librería) |

### Criterio aplicado a los “verde falso” de `browser_e2e`

Mecanismo **sin cambios** (`browser_e2e.rs`: `if !enabled() { return; }` en 7 tests →
verde sin ejecutar sin `AMATL_BROWSER_E2E=1`), pero deliberado y documentado en el
header del fichero (“the only legitimate skip”) y con suite fail-closed tras opt-in.
Clasificado **NOT_A_BUG (decisión) / riesgo CI conocido**: enmascara regresiones de
UI en el gate normal. Opción barata: `#[ignore]` para hacer el skip visible.

### Re-verificación de hallazgos previos (2026-11-13)

| Previo | Estado |
|---|---|
| `GET /deep` contradecía POST-only | **CORREGIDO** (solo `post(deep_post)`; GET → 405 con `allow: POST`, test fijado) |
| Docs “5 tools MCP” siendo 6 | **CORREGIDO** (docs y memoria = 6, `tools/list` en vivo = 6) |
| Supply chain `cargo audit` (5 vulns + unsound) | **MAYORMENTE CORREGIDO**: hoy 1 (`libsqlite3-sys` RUSTSEC-2026-0459, suprimido de forma acotada en `deny.toml` con ADR-017 + plan de salida) + warning `chrono` unmaintained. ENVIRONMENTAL |
| `browser_e2e` verde falso | Sin cambios (ver arriba) |
| Sin unit tests en `providers.rs`/`model.rs`/`normalize.rs`/`config` | Sigue abierto — precisamente donde viven B1 y B4 |

## Lo que se revisó y está correcto (resumen de cobertura)

Auth digest en tiempo constante, expiraciones, scopes fail-closed con default
Admin en rutas no declaradas, antispoofing MCP, CSP/X-Request-ID en todas las
respuestas, Host/Origin/CORS, rate limit por peer address, límites de cuerpo/
cabeceras/deadline, `config_write_lock` sin lost updates, `POST /reload` atómico,
backup con `Idempotency-Key`, SSRF completo en `security.rs`/`fetch.rs` (DNS
pinning sin TOCTOU), SQL 100 % parametrizado, secretos `Zeroizing` sin fugas en
errores/logs, UI sin XSS (todo `textContent`), i18n 223/223 claves es/en, contratos
JS validados (`schema_version` + linaje de evidencia), budget/reintentos acotados,
`answer.rs` con citas limitadas y validadas, `canonical.rs` idempotente, fechas,
`evidence.rs` con límites de char seguros, `child_process` con env-scrub y
`kill_on_drop`, sin `unsafe`, sin pánicos alcanzables. Detalle completo en los dos
informes de frente.

## Corrección mínima propuesta (orden sugerido; nada aplicado)

1. **B1**: re-validar tras `decode_html_entities` en `clean_text` (o decodificar antes).
2. **B4**: `#[serde(deny_unknown_fields)]` en secciones de seguridad (`data_policy`, `server`, `providers`, `inference`) — el proyecto se declara fail-closed.
3. **B2**: normalizar el fragmento antes de `visited.insert` (`set_fragment(None)`).
4. **BH-02**: clasificar errores con `error.status`/`error.code` (client.js ya los expone).
5. **BH-01**: UI envía `tools: MCP_TOOLS` cuando se concede scope `mcp` (o checkboxes por tool); idealmente el servidor rechaza cliente `mcp` con `tools: []`.
6. **B3/BH-04**: `saved_document_get_by_id(id)` parametrizado en storage y su uso en `Show`.
7. **B5**: contabilizar redirects seguidos también en fallo (`FetchAttempt{redirects_followed, outcome}`).
8. **B7**: recapturar `now` tras el `sleep` de Crawl-delay (o `timeout_at(deadline, …)`).
9. **B6**: cotas superiores en `validate_cache` + `checked_add` / casts saturantes.
10. **BH-03**: clamp de `page` al total en el render/store.

Regla del repo: cada fix va con su test de regresión **antes** del fix.