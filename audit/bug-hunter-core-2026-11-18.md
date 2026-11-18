# Bug hunt — amatl-core + amatl-cli/src/main.rs

- **Fecha:** 2026-11-18
- **Alcance:** `crates/amatl-core/src/` (toda la lógica de producto) y `crates/amatl-cli/src/main.rs`. Solo lectura: no se modificó ningún archivo del repo, no se crearon tests dentro del repo, no se tocaron `amatl.toml`/`.env`/SQLite.
- **Metodología:** bug-hunter (evidencia por encima de suposición). Trazado de caminos de ejecución en los módulos de alto riesgo, búsqueda de patrones de pánico/desbordamiento/TOCTOU/fail-open, y reproducción independiente mediante un crate scratch en `/tmp/user/1000/opencode/repro` que depende de `amatl-core` por path (corre con `CARGO_TARGET_DIR=/home/panda/amatl/target cargo run --offline`, reutilizando solo caché de compilación).
- **Base de pruebas:** `cargo test -p amatl-core` → 406 tests unitarios + integración, **0 fallos** (línea base sana; los fallos encontrados no están cubiertos por tests existentes).

**Resumen de hallazgos:** 3 CONFIRMED_BUG, 2 PROBABLE_BUG, 2 POTENTIAL_BUG.

| ID | Clasificación | Título |
|----|---------------|--------|
| B1 | CONFIRMED_BUG | Entidades HTML decodificadas *después* de la validación permiten inyectar chars de control (ESC/NUL) en títulos y snippets |
| B2 | CONFIRMED_BUG | El frontier de crawl duplica el mismo documento por variantes de fragmento (#a/#b/#c) |
| B3 | CONFIRMED_BUG | `amatl saved show --id N` falla para ids fuera de los 200 documentos más recientes |
| B4 | PROBABLE_BUG | Claves desconocidas/typo en `amatl.toml` se ignoran en silencio: un typo en `[data_policy]` deja la política en `standard`/`governed` con egress abierto |
| B5 | PROBABLE_BUG | Los redirects seguidos por fetches que *fallan* nunca se cargan al presupuesto `remaining_redirects` |
| B6 | POTENTIAL_BUG | Conversiones `as i64` sin cota en la aritmética de TTL del caché (alcanzable solo con valores ≥ 2^63 vía API Rust; TOML los rechaza) |
| B7 | POTENTIAL_BUG | `fetch_timeout` de Deep se calcula con un `now` anterior a la espera de `Crawl-delay`: Deep puede exceder su deadline hasta 5 s |

---

## B1 — Entidades HTML decodificadas después de la validación permiten inyectar caracteres de control

- **Clasificación:** CONFIRMED_BUG
- **Módulo/archivo:líneas:** `crates/amatl-core/src/normalize.rs:133-173` (`clean_text` 133-146, `decode_html_entities` 148-173)
- **Síntoma observable:** un resultado de proveedor (respuesta real de SearXNG/Marginalia, o cualquier contenido web capturado por un proveedor) puede llevar en `title`/`snippet`/`author`/etc. secuencias que, tras "limpiarse", contienen bytes de control: `ESC` (0x1B — inyección de secuencias ANSI en la terminal), `NUL` (0x00) y el propio U+FFFD que `clean_text` se compromete a rechazar. El CLI imprime esos títulos sin escapar (`main.rs:1228-1235`, `print_search`), y la misma cadena llega al JSON público y a la UI.

**Evidencia (reproducción ejecutada).** Scratch crate contra `amatl-core`, un `ProviderItem` con:

- `title = "SAFE&#27;&#91;&#51;&#49;&#109;PWNED"` (entidades para `ESC [ 3 1 m`)
- `snippet = "cero&#0;fin"` (entidad para NUL)

llamando a `amatl_core::normalize::normalize`:

```text
[T1] degradations = []
[T1] title bytes  = [83, 65, 70, 69, 27, 91, 51, 49, 109, 80, 87, 78, 69, 68]
[T1] snippet bytes= [99, 101, 114, 111, 0, 102, 105, 110]
[T1] ESC en title tras clean_text: true
[T1] NUL en snippet tras clean_text: true
```

El título contiene `ESC [ 3 1 m` literalmente, el snippet contiene un NUL, y **no se registra ninguna degradación**.

**Cadena síntoma → mecanismo → causa raíz.** Título/snippet con chars de control en la salida → `decode_html_entities` convierte `&#27;`, `&#0;`, `&#65533;` en caracteres reales *después* de que `clean_text` ya hizo su chequeo → `clean_text` valida el texto crudo (`value.contains(['\0','\u{fffd}'])`, `is_control() && !is_whitespace()`) y solo después llama a `decode_html_entities`; el texto ya validado nunca se re-valida.

**Impacto.** Inyección ANSI/CSI en terminales de operadores a través de resultados de proveedores hostiles (ataque tipo "terminal escape injection" mediante resultados de búsqueda), corrupción de campos con NUL (problemas aguas abajo al tratarlos como C-strings o al volcar a herramientas externas) y violación directa del contrato documentado de `clean_text` ("rechaza caracteres de control y U+FFFD"). Un proveedor comprometido o una respuesta MITM/inyectada en un metabuscador bastan como vector.

**Reproducción.** Ver arriba (scratch T1, `/tmp/user/1000/opencode/repro/src/main.rs`, `test_normalize_control_chars`). También funciona con `&#x1b;` y produce U+FFFD con `&#xfffd;`.

**Corrección mínima sugerida (sin aplicar).** Decodificar entidades *antes* de la validación y validar el resultado ya decodificado (mover `decode_html_entities` al inicio de `clean_text`, o repetir el chequeo de control/`\0`/U+FFFD sobre el texto decodificado). Alternativa mínima: rechazar el campo si la decodificación introduce cualquier char que el chequeo habría rechazado.

---

## B2 — El frontier de crawl duplica el mismo documento por variantes de fragmento

- **Clasificación:** CONFIRMED_BUG
- **Módulo/archivo:líneas:** `crates/amatl-core/src/deep.rs:438-452` (inserción en `visited` usando la URL completa con fragmento), `deep.rs:662-676` (`child_result` elimina el fragmento *después* de insertar), `deep.rs:171-174` (inicialización de `visited`)
- **Síntoma observable:** si una página descubierta contiene enlaces al mismo destino con distinto fragmento (`/page#a`, `/page#b`, `/page#c` — anclas, navegación SPA, "toc"), Deep descarga esa misma página **una vez por variante**, devuelve varios `Document` con **idéntico `canonical_url` e idéntico `search_result_id`**, y consume presupuesto de fetch/red por cada duplicado.

**Evidencia (reproducción ejecutada).** Scratch T2: `DeepOrchestrator` real con un `Fetcher` falso que responde `/start` con:

```html
<a href="/page#a">a</a><a href="/page#b">b</a><a href="/page#c">c</a><a href="/other">o</a>
```

Resultado:

```text
[T2] URLs fetchadas (orden): ["https://example.com/start", "https://example.com/page",
     "https://example.com/page", "https://example.com/page", "https://example.com/other"]
[T2] documentos devueltos: 5 canónicos=[".../start", ".../page", ".../page", ".../page", ".../other"]
[T2] fetches de /page: 3 (correcto sería 1)
```

**Cadena síntoma → mecanismo → causa raíz.** 3 descargas y 3 documentos duplicados → el dedupe del frontier usa `visited.insert(link.as_str())` donde `link` conserva el fragmento, mientras el hijo encolado pasa por `child_result`, que ejecuta `url.set_fragment(None)` → la clave de dedupe y la identidad canónica del documento usan representaciones distintas de la misma URL; `#a`, `#b`, `#c` son claves distintas para el mismo recurso.

**Impacto.** Presupuesto de Deep malgastado (hasta 32 enlaces/tomado por página, todos `take(32)`) en descargas redundantes — directamente en contra de la función del `DeepBudget` — y respuesta `DeepResponse` con documentos duplicados cuyo `search_result_id` colisiona (afecta a `original_ranks`, ranking_v2 y evidencia). Páginas con muchas anclas (documentación, wiki) multiplican el gasto.

**Reproducción.** Ver arriba (scratch T2). Cualquier página real con menú de anclas la reproduce.

**Corrección mínima sugerida (sin aplicar).** Normalizar el enlace antes de consultarlo contra `visited` (aplicar `set_fragment(None)` en `discover_links` o antes del `visited.insert`), dejando que la clave del conjunto sea exactamente la forma canónica que se encola. Idempotente con el comportamiento actual de `child_result`.

---

## B3 — `amatl saved show --id N` falla para ids fuera de los 200 documentos más recientes

- **Clasificación:** CONFIRMED_BUG
- **Módulo/archivo:líneas:** `crates/amatl-cli/src/main.rs:699-707` (`SavedCommand::Show`), `crates/amatl-core/src/storage.rs:1851-1884` (`saved_document_list`, única vía de lectura por lotes; no existe lookup por id)
- **Síntoma observable:** con más de 200 documentos guardados, `amatl saved show --id N` imprime `no saved document with id N` para cualquier id que no esté entre los 200 más recientes, aunque el documento existe y `saved delete --id N` sí lo encuentra y lo borra.

**Evidencia (reproducción ejecutada).** Scratch T3 contra `SqliteStorage` real: se insertan 201 documentos guardados vía `saved_document_put`; luego se replica *exactamente* la lógica de `SavedCommand::Show` (`saved_document_list(200, 0).find(id)`):

```text
[T3] total guardados = 201, id más antiguo = 1
[T3] ¿`saved show --id 1` lo encuentra? false
```

**Cadena síntoma → mecanismo → causa raíz.** El comando falla con "no existe" sobre un id válido → la implementación emula un "get por id" escaneando los 200 primeros de `saved_document_list(200, 0)` (orden `saved_at DESC, id DESC`) → en el dominio no hay `saved_document_get_by_id` (sí `saved_document_get` por URL/hash/extractor), y el CLI eligió un escaneo de ventana fija como sustituto.

**Impacto.** Error falso-negativo en una superficie de operación; el usuario no puede leer documentos antiguos por id (formato sin `--json` imprime ids, invitando al flujo `show`). No hay corrupción de datos: `delete` usa `saved_document_delete(id)` real y funciona.

**Reproducción.** Scratch T3 (`test_saved_show_beyond_200`). Alternativa manual: guardar 201 documentos y ejecutar `saved show --id` del primero.

**Corrección mínima sugerida (sin aplicar).** Añadir `SqliteStorage::saved_document_get_by_id(id)` (`SELECT ... WHERE id = ?` parametrizado, con `write_lock` innecesario) y usarlo en `SavedCommand::Show`. El lookup por URL/hash puede quedar intacto.

---

## B4 — Claves desconocidas/typo en `amatl.toml` ignoradas en silencio (fail-open en `data_policy`)

- **Clasificación:** PROBABLE_BUG (el mecanismo está reproducido; que "ignorar claves desconocidas" sea incorrecto es un juicio de diseño — el resto del proyecto se declara fail-closed y no existe ni aviso ni error)
- **Módulo/archivo:líneas:** `crates/amatl-core/src/config/mod.rs:277-290` (`Config::from_toml` = `toml::from_str` sin `deny_unknown_fields`; `validate()` tampoco detecta claves desconocidas)
- **Síntoma observable:** un `amatl.toml` con un typo en una clave de seguridad — p. ej. `[data_policy] profle = "isolated"` y `egres = "deny"` — carga sin error, `validate()` devuelve `Ok`, y el proceso queda con `profile=Standard`, `egress=Governed`, **egress de red permitido** mientras el operador cree que está en modo aislado.

**Evidencia (reproducción ejecutada).** Scratch T4, `Config::from_toml` con ese TOML exacto:

```text
[T4] parseo OK pese al typo; profile=Standard egress=Governed inference=Disabled
[T4] allows_network_egress() = true (el operador escribió isolated/deny)
[T4] validate() = true
```

Ningún warning en stderr, ninguna degradación.

**Cadena síntoma → mecanismo → causa raíz.** Política de seguridad no aplicada y sin aviso → serde ignora campos desconocidos por defecto y `validate()` no compara las claves presentes con el esquema → `Config::from_toml` no usa `#[serde(deny_unknown_fields)]` ni ninguna pasada de "claves desconocidas", contra la filosofía fail-closed que el propio código documenta para `data_policy` (`config/data_policy.rs:82-90`).

**Impacto.** Un typo en cualquier sección silencia la configuración pretendida; en `[data_policy]`, `[server]` (p. ej. `no_aut`) o `[providers]` el efecto es un fail-open de seguridad: egress abierto, servidor sin auth, claves de credencial no leídas (`credential_env` mal escrito → proveedor sin credencial o peor, resolución por defecto). También mitiga el valor de `amatl config`/`doctor`, que muestran el estado efectivo pero nadie compara con la intención escrita.

**Reproducción.** Scratch T4 (`test_config_unknown_keys`). Cualquier clave inventada en cualquier sección reproduce el silencio.

**Corrección mínima sugerida (sin aplicar).** Añadir `#[serde(deny_unknown_fields)]` como mínimo a `DataPolicyConfig` y a las secciones con implicaciones de seguridad (`server`, `providers`, `inference`), o una pasada post-carga en `validate()` que recorra las clades del TOML con `toml_edit` y rechace (o al menos reporte como `ConfigError`) las claves que no pertenezcan al esquema conocido. Rechazar es preferible a avisar dada la postura del proyecto.

---

## B5 — Los redirects seguidos por fetches que fallan nunca se cargan al presupuesto `remaining_redirects`

- **Clasificación:** PROBABLE_BUG (mecanismo inequívoco en el código; no se reprodujo con un servidor real de redirects — ver "Reproducción")
- **Módulo/archivo:líneas:** `crates/amatl-core/src/deep.rs:285-294` (`consume_fetch(size, redirects)` solo en éxito), `deep.rs:226-230` (tope por petición = `remaining_redirects.min(max_redirects)`), `crates/amatl-core/src/fetch.rs:187-195` (el error `RedirectLimit` se emite tras haber ya seguido N saltos)
- **Síntoma observable:** `DeepBudgetSnapshot.remaining_redirects` solo desciende cuando un fetch *termina bien*. Un fetch que agota `max_redirects` y falla (`RedirectLimit`), o que falla por timeout/red tras seguir varios saltos, ha producido tantas peticiones HTTP de redirect como saltos siguió, pero el contador global no se mueve. Con `max_fetches=F` y presupuesto `R` de redirects, el tráfico real de redirects puede llegar a `F × R` en lugar de `R`.

**Evidencia.** Lectura de código:
- `deep.rs:215-224` reserva un fetch; `deep.rs:285-294` hace `consume_fetch(fetched.size, fetched.redirect_chain.len() as u32)` **solo** en el camino `Ok`; en el camino `Err` (265-283) solo hay `release_fetch` para ciertos errores y ningún cargo de redirects.
- `fetch.rs:187-195`: ante el redirect `N+1` con `redirects.len() >= max_redirects` se devuelve `Err(FetchError::RedirectLimit)` habiendo ya realizado `max_redirects` peticiones de redirect que nunca se contabilizan.
- El tope por petición (`remaining_redirects.min(max_redirects)`) limita cada fetch aislado pero no al conjunto, contra el invariante "el orquestador es el único dueño del Budget global".

**Cadena síntoma → mecanismo → causa raíz.** El snapshot del presupuesto reporta más redirects de los disponibles y el límite global se excede hasta `F×R` → el cargo de redirects ocurre solo en el caso de éxito (`fetched.redirect_chain` no existe en el error) → `consume_fetch` no tiene forma de saber cuántos saltos siguió un fetch fallido.

**Impacto.** Egress no gobernado por el presupuesto que se declara dueño del mismo (atacante con páginas que redirigen en bucle/encadenado y provocan timeouts agota tráfico por encima del presupuesto); métricas/`budget_snapshot` inexactas para el operador.

**Reproducción (pendiente).** Requiere un `SafeFetcher` contra un servidor local con cadenas de redirect que terminen en timeout o en redirect infinito, y comparar el número de GETs observados con `remaining_redirects`. Los tests existentes de `fetch.rs` ya levantan servidores locales en `127.0.0.1`, así que la reproducción es factible sin tocar el repo (scratch).

**Corrección mínima sugerida (sin aplicar).** Informar los saltos seguidos también en caso de error (p. ej. un `FetchError` enriquecido o un `FetchAttempt { redirects_followed, outcome }`) y hacer `consume_fetch(0, redirects_followed)` en el camino de error; o reservar los redirects por adelantado en el presupuesto por petición y liberar solo los no usados. Mantener el tope por petición actual.

---

## B6 — Conversiones `as i64` sin cota en la aritmética de TTL del caché

- **Clasificación:** POTENTIAL_BUG (reproducido con la API Rust; *no alcanzable* con los valores que `amatl.toml` puede expresar, ver evidencia)
- **Módulo/archivo:líneas:** `crates/amatl-core/src/storage.rs:1079, 1136, 1262, 1296, 1310, 1332-1334` (`ttl_seconds as i64`, `ttl_seconds + stale_while_revalidate_seconds as i64`), `storage.rs:1145-1151, 1182-1201` (`max_entries as i64`, `max_bytes as i64`); cota ausente en `crates/amatl-core/src/config/persistence.rs:393-402` (`validate_cache` solo exige `!= 0`)
- **Síntoma observable (con valores ≥ 2^63):** una entrada de caché recién escrita nunca se considera fresca (o el `put` la borra a sí misma): `ttl_seconds as i64` se vuelve negativo y el corte `now.saturating_sub(negativo)` = `now + |x|` hace que `created_at >= corte` sea siempre falso.

**Evidencia.**
1. Reproducción T5 (scratch): `ProviderSearchCachePolicy { ttl_seconds: u64::MAX, .. }` → `cache.put(...)` y `cache.get(...)` inmediatamente después:

   ```text
   [T5] entrada recién escrita con ttl_seconds=u64::MAX, ¿cache hit? false
   ```

2. Mitigación de alcanzabilidad verificada en la misma ejecución:

   ```text
   [T4b] ttl_seconds=18446744073709551615 rechazado: number too large to fit in target type
   [T4b] ttl_seconds=9223372036854775807 parsea OK -> ... (validate ok)
   ```

   TOML expresa enteros con signo de 64 bits: un valor ≥ 2^63 **no puede escribirse** en `amatl.toml`, y el camino de escritura administrativa (`signed_integer`, `config/mod.rs:84-95`) rechaza explícitamente valores por encima de `i64::MAX`. Con el máximo expresable (`i64::MAX`), la semántica resultante ("TTL infinito") es la esperada. El caso queda por tanto limitado a embebidos que construyan `ProviderSearchCachePolicy`/`DocumentCachePolicy` directamente con `u64` sin pasar por `Config`.

**Cadena síntoma → mecanismo → causa raíz.** Caché inútil o entradas borradas al escribir → casts `u64 as i64` sin comprobar y suma `ttl + stale` sin `checked_add` → `validate_cache` no impone cota superior (a diferencia de lo que el propio `config/persistence.rs:333-335` promete: "Every numeric limit is bounded in *both* directions, unconditionally").

**Impacto.** Bajo en las superficies actuales (CLI/UI/API/MCP pasan por `Config`); media para cualquier uso futuro de la API de la librería con valores arbitrarios.

**Reproducción.** Scratch T5 (`test_cache_ttl_wrap`). El caso TOML no reproduce (correctamente limitado por el parser).

**Corrección mínima sugerida (sin aplicar).** Completar `validate_cache` con cotas superiores (`ttl_seconds`, `stale_while_revalidate_seconds`, `max_entries`, `max_bytes` ≤ `i64::MAX/2` o límites operativos tipo 10 años/10^9) y usar `saturating as i64` con `min(i64::MAX as u64)` o `checked_add` en `storage.rs:1334`.

---

## B7 — `fetch_timeout` de Deep calculado con un `now` anterior a la espera de `Crawl-delay`

- **Clasificación:** POTENTIAL_BUG
- **Módulo/archivo:líneas:** `crates/amatl-core/src/deep.rs:176` (`now` capturado al inicio del ciclo), `deep.rs:195-202` (`tokio::time::sleep(wait)` tras el chequeo de robots), `deep.rs:231-234` (`fetch_timeout = min(timeout_ms, deadline - now)` con el `now` ya obsoleto)
- **Síntoma observable:** tras respetar un `Crawl-delay` de `W` ms, el fetch posterior recibe un presupuesto de tiempo calculado como si la espera no hubiera ocurrido; la petición puede terminar hasta `W` ms después del deadline global de Deep. La cota es `MAXIMUM_CRAWL_DELAY_MS = 5 s` (`robots.rs:31-33`).

**Evidencia.** Lectura de código: el comentario en `deep.rs:194` ("Politeness never outlives the Deep deadline") solo controla que la *espera* no cruce el deadline; el timeout del fetch siguiente se deriva de `now` (línea 176), anterior al `sleep`. `fetch_inner` crea su propio deadline relativo (`fetch.rs:154`) con ese `timeout_ms`, y el bucle de Deep no envuelve el fetch en `timeout_at(deadline, ...)` (a diferencia de `extract`/`render`, líneas 326/340, que sí).

**Cadena síntoma → mecanismo → causa raíz.** Deep excede su deadline acotado → `fetch_timeout` usa un timestamp anterior a la demora de cortesía → `now` no se recaptura tras `sleep`.

**Impacto.** Bajo: sobrerregla acotada a 5 s por link con `Crawl-delay`, en un escenario ya poco frecuente (robots con delay y crawl profundo). Rompe la garantía "remaining_deep_ms es el deadline total" que `deep_from_search` (`service.rs:1348-1357`) deduce restando el tiempo de Search.

**Reproducción (pendiente).** Scratch con `RobotsCache` real cuyo fetcher entregue `robots.txt` con `Crawl-delay: 1` y mida `elapsed_ms` del `DeepResponse` frente al deadline; determinista y barato, no llegué a ejecutarlo.

**Corrección mínima sugerida (sin aplicar).** Recalcurar el instante actual junto a `fetch_timeout` (`let now = tokio::time::Instant::now();` justo antes de la línea 231), o envolver el `self.fetcher.fetch(...)` en `tokio::time::timeout_at(deadline, ...)`.

---

## Re-verificación de los hallazgos abiertos del 2026-11-13

- **Supply chain (`cargo audit`):** ejecutado hoy sobre `Cargo.lock`. Estado actual: **1 vulnerabilidad** — `RUSTSEC-2026-0459` (`libsqlite3-sys 0.30.1`, CVE-2026-119375, sin salida desde `sqlx 0.8.6`) más el warning `chrono unmaintained` (`RUSTSEC-2026-0543`). Los advisories de `rmcp`, `quinn-udp` y `rustix` ya **no aparecen**. El de `libsqlite3-sys` está suprimido de forma acotada y documentada (`deny.toml` `ignore = ["RUSTSEC-2026-0459"]` + ADR-017 + `docs/security/supply-chain.md`), con lo que la deuda es conocida y con plan de salida. Clasificación: **ENVIRONMENTAL** (fuera del alcance de código; sin hallazgo nuevo).
- **`browser_e2e` verde en vez de skip:** vive en `crates/amatl-server/tests/browser_e2e.rs` (fuera de alcance). La revisión rápida muestra que hoy documenta y afirma el único skip legítimo sin `AMATL_BROWSER_E2E=1`; el patrón "early return verde" parece remediado. **UNRELATED/fuera de alcance**.
- **Tests unitarios ausentes en providers.rs/model.rs/normalize.rs/config:** sigue sin cubrirse (mención única): se observa que precisamente `normalize.rs` y `config` son donde residen B1/B4; tests de esas unidades los habrían detectado. **POTENTIAL** (calidad, no es bug por sí mismo).

---

## Qué revisé y está correcto

Cobertura verificada lectura-ejecución (además de los módulos con hallazgos):

- **`budget.rs`**: reservas/cargas/liberaciones de `Budget` y `DeepBudget` sin expansión; `release_*` acotados por máximos; límite duro de 2 subqueries y de coste; test unitario alineado.
- **`execution.rs` (SearchOrchestrator)**: semáforos global/por-proveedor, deadline por tarea con `timeout_at`, reintentos acotados (`max_retries.min(2)` y `validate_execution` ≤ 2; jitter ≤ 1000 ms validado; `1_u64 << attempt` inalcanzable ≥ 64), `join` de tareas con relleno de fallidos, clasificación de errores de provider correcta (`CONCURRENCY_DEADLINE_MESSAGE` no abre el circuit breaker).
- **`service.rs`**: límites por superficie (MCP más restrictivo) con divisiones protegidas (`max_fetches.max(1)`), `validate_deep_fetch_cap` rechaza 0 y excesos, paginación server-side sin desbordes (`clamp`, `saturating_*`, cota superior), merge del refined search sin duplicar provenance/status, historial best-effort, credenciales efímeras `Zeroizing` sin logging, sonda de credencial fail-closed con egress denegado.
- **`errors.rs`/`ServiceError`**: catálogo estable; la cobertura de `AnswerUnavailable` ("…or its response was not grounded") hace que mapear `Ungrounded` ahí sea diseño, no error.
- **`answer.rs`**: extracción de citas y `strip_invalid_citations` con manejo correcto de límites de char (test de regresión multibyte incluido); respuesta sin citas rechazada (`Ungrounded`); fuentes acotadas.
- **`security.rs`**: SSRF — esquemas, credenciales embebidas, nombres internos/single-label, IPv4 (privada, loopback, link-local, broadcast, documentación, 0/8, 240/4, CGNAT 100.64/10, 192.0.0/24, 198.18/15), IPv6 (mapeada y compatible IPv4, ULA, link-local, 2001:db8, NAT64 64:ff9b::/96, 6to4 2002::/16). (Brecha teórica menor no reportada: `fec0::/10` site-local y 3fff::/20 documentación sólo-dependientes de la versión de Rust.)
- **`fetch.rs`**: resolución DNS validada y cliente pinneado a las direcciones validadas (sin TOCTOU DNS), redirects re-validados con `validate_deep_url`, cabeceras de salida por allowlist, límite de bytes en streaming, clientes acotados a 64 con evicción, `no_proxy`/`referer(false)`, Debug redactado.
- **`providers/http.rs`**: `sanitized_url` + `Debug` manual sin secretos; errores de transporte clasificados y sin URL cruda en logs.
- **`providers/searxng.rs`/`marginalia.rs`/`registry.rs`**: construcción de consultas sin interpolación peligrosa (params/cabeceras separados); credenciales `Zeroizing` desde `credential_env`, uso correcto de `unresponsive_engines` tipado; errores mapeados a `ProviderErrorKind` estables sin fugas de URL con claves.
- **`canonical.rs`**: transformaciones v1 seguras e idempotentes, preservación de escapes `%` malformados con `Degraded`, eliminación de solo-fragmento-vacío y params de tracking (con tests). (Observación descartada: `normalize_percent_hex` usa `byte as char`, potencialmente corruptor con no-ASCII, pero `url::Url::as_str()` es siempre ASCII → no alcanzable.)
- **`cache.rs`/`storage.rs` (resto)**: claves de caché incluyen `adapter_version` + filtros estructurados; evicción por entradas y por bytes con running-sum en una sentencia; SQL 100% parametrizado (sin concatenación de valores: `query!`/`bind` en todas las sentencias; el único `format!` es un patrón `LIKE` escapado en `search_history_filter`); `write_lock` para escritores, lecturas sin lock documentadas; listas descendentes/ascendentes restauradas por el llamador.
- **`query.rs`**: tokenización con comillas, validación de dominios/fechas (año bisiesto Gregoriano correcto, día/mes validados), rango `after >= before` invalidado con warning, `token[1..]` seguro por prefijo ASCII.
- **`text.rs`**: NFKC + casefold con casos ß/ς correctos; folding de confusables con invariancias verificadas por tests (incl. sigma final vs `NormalizedText`).
- **`normalize.rs` (fechas)**: RFC3339, ISO8601 con offset compacto, naive anclado a UTC y fechas inválidas rechazadas (tests de regresión presentes).
- **`evidence.rs`**: todos los slices `content[start..end]` derivan de `char_indices`/`floor_char_boundary`/`find` sobre bytes ASCII → límites de char seguros; `unwrap` solo en tests.
- **`extract.rs`/`child_process.rs`**: env scrub por allowlist (canary test incluido), `kill_on_drop`, `stdout.take(max+1)` con rechazo por `OutputLimit`, timeouts acotados, reintentos `ETXTBSY` acotados y deterministas; `probe_version` limitado a 256 bytes y sin chars de control.
- **`deep.rs` (resto)**: presupuesto de fetch/bytes/browser/subqueries aplicado también al DOM renderizado; 304 con revalidación correcta; robots solo para links descubiertos (depth>0); extractor ajustado a `remaining_deep_ms`; enlaces restringidos a http(s) y mismo host.
- **`robots.rs`**: parsing de grupos por user-agent con precedencia específica, `Crawl-delay` acotado a 5 s, cache por origen con evicción de uno.
- **`document_cache.rs`/`circuit.rs`/`telemetry.rs`**: TTL con `saturating_sub`, contadores atómicos (cero overflow en tasas), breaker con cooldown por `saturating_add`, recuperación de mutex con log de evento de seguridad.
- **`model.rs`**: invariantes `Rank`/`RankingScore` con fallo tipado y `bounded` seguro ante NaN/infinito; `SCHEMA_VERSION` único usado por todos los constructores públicos (sin deriva encontrada).
- **`amatl-cli/src/main.rs` (resto)**: mapa de errores a catálogo en stderr separado del JSON en stdout; `serve` revalida con la dirección efectiva (`validate_server_for_listener` en el server); historial/saved/db con límites `clamp(1, 200)`; sin `unwrap` salvo tests.
- **`config/*` (resto)**: validación cruzada `isolated` → `deny` + loopback + sin renderer; `deny` → sin providers ni inferencia remota; numerales de ejecución/persistencia/inferencia acotados en ambos extremos; escritura con `toml_edit` preservando comentarios, modo 0600 y rename atómico; patches de `data_policy` como unidad. (Hallazgo B4 es precisamente la ausencia de rechazo de claves desconocidas; el resto del esqueleto es sólido.)
- **Búsquedas de pánicos**: `unwrap`/`expect` en código de producción solo en invariantes justificados (`answer.rs:286` documentado y no alcanzable, `lock_clients` con recuperación de poisoning) o en `#[cfg(test)]`; sin `unsafe` en todo el crate; sin `panic!`/`todo!` alcanzables.

---

## Apéndice — reproducciones

Todas se ejecutaron desde `/tmp/user/1000/opencode/repro` (crate scratch, sin tocar el repo) con:

```bash
cd /tmp/user/1000/opencode/repro
CARGO_TARGET_DIR=/home/panda/amatl/target cargo run --offline
```

Salida resumen final observada:

```text
[T1] ESC en title tras clean_text: true          → BYPASS CONFIRMADO
[T2] fetches de /page: 3 (correcto sería 1)      → DUPLICADO CONFIRMADO
[T3] ¿`saved show --id 1` lo encuentra? false    → FALLO CONFIRMADO
[T4] allows_network_egress() = true              → SILENCIO CONFIRMADO (typo en data_policy)
[T5] cache hit tras put con ttl u64::MAX: false  → FALLO CONFIRMADO (API Rust; no vía TOML)
```