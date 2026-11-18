# Auditoría bug-hunter (solo lectura): amatl-server · amatl-ui · amatl-cli

- **Fecha:** 2026-11-18
- **Alcance:** `crates/amatl-server/src/` (routes.rs, mcp.rs, lib.rs, tests.rs), `crates/amatl-server/tests/`, `crates/amatl-ui/` (app.js, client.js, store.js, i18n.js, index.html, styles.css y el embebido Rust), `crates/amatl-cli/src/main.rs`.
- **Método:** trazado estático de rutas de petición (auth, scope, MCP, rate limit, límites de cuerpo/tiempo, CORS/Host, assets estáticos), revisión JS (XSS, i18n, carreras de estado, contratos JSON), revisión del adaptador CLI, y **verificación en vivo** con servidor real (mock) sobre configuración de sonda en `/tmp` (sin tocar `amatl.toml`, `.env` ni SQLite del repo). Ningún archivo del repositorio fue modificado.
- **Suite:** `cargo test -p amatl-server --locked` → **verde** (150 unit/integration + 8 browser_e2e + 2 soak ignorados). Se recompiló `amatl-cli` antes de las sondas porque el binario de `target/debug` estaba desactualizado respecto a los fuentes (ver «Notas»).

---

## HALLAZGOS

### BH-01 — Los clientes creados desde la UI quedan sin ninguna herramienta MCP: el scope «MCP» es un control muerto

- **Clasificación:** CONFIRMED_BUG
- **Archivo:líneas:** `crates/amatl-ui/assets/app.js:1884` (`const body = { id, scopes, tools: [] };`), `crates/amatl-ui/assets/app.js:1869-1909` (`createServerClient`), `crates/amatl-server/src/mcp.rs:93-109` (`authorize`), `crates/amatl-server/src/lib.rs:453-455` (`allows_tool`), `crates/amatl-server/src/lib.rs:1673-1681` (`CreateServerClientParams`)
- **Síntoma observable:** un operador crea un cliente desde el panel «Clientes y tokens», marca la capacidad «MCP» y recibe el token con «Cliente creado» y sin ningún aviso. Ese cliente llega a `/mcp` (scope `mcp` concedido) pero **todas** las llamadas `tools/call` devuelven `scope_denied` (`isError: true`): la credencial no puede usar search, deep_search, fetch, providers, status ni answer. La UI no ofrece ningún control para fijar `tools`, ni en el alta ni después (no hay superficie PATCH).
- **Evidencia:**
  - `app.js:1884` envía siempre `tools: []` (literal, sin ramificación por scope).
  - `ClientIdentity::allows_tool` (`lib.rs:453-455`) es un `any` sobre la lista: `[]` deniega **todas** las herramientas.
  - `Config::validate` (core `config/server.rs`) permite `tools: []` con scope `mcp` (solo rechaza tools *sin* scope mcp y tools desconocidos), así que la configuración es válida pero inerte.
  - **Sonda en vivo** (servidor mock + API admin): creación exacta de lo que publica el formulario (`{"id":"ui_created2","scopes":["search","mcp"],"tools":[]}`) → `tools/call search` con esa credencial responde `{"code":"scope_denied","message":"credential lacks the required capability"}`. Un cliente declarado a mano con `tools=["search"]` sí puede llamar `search` y se le deniega `fetch` (controles cruzados correctos).
  - En contraste, el cliente «default» (token de entorno) recibe las seis herramientas (`lib.rs:482`) y los tests del servidor declaran `tools` explícitos (`tests.rs:3420-3438`), nunca el caso de la UI.
- **Cadena síntoma → mecanismo → causa raíz:** checkbox «MCP» activo y cliente creado sin error visible → `resolve_clients` monta `AuthorizedClient { tools: [] }` y `McpSurface::authorize` llama `allows_tool` por herramienta, que sobre lista vacía siempre falla → causa raíz: la UI fija `tools: []` («la capacidad se concede explícitamente», pero el único formulario que existe nunca la concede) y no existe superficie de UI para la allowlist.
- **Impacto:** cualquier cliente destinado a agente MCP creado desde la UI queda inutilizable para MCP sin diagnóstico; el operador debe editar `amatl.toml`/PATCH por API para arreglarlo. Riesgo funcional (promesa incumplida del formulario), no de seguridad: no amplía capacidades, las recorta.
- **Reproducción:**
  1. Servir con un token admin; abrir la UI → panel «Clientes y tokens».
  2. Crear cliente con id cualquiera y capacidades «Search» + «MCP».
  3. `POST /mcp` con JSON-RPC `tools/call {"name":"search",...}` y el token recién creado → `scope_denied`.
- **Corrección mínima sugerida (sin aplicar):** en `createServerClient`, cuando el conjunto de scopes incluya `mcp` y no haya selección de herramientas, enviar `tools: MCP_TOOLS` (el vocabulario completo que `Config::validate` ya admite), o añadir un grupo de checkboxes por herramienta en el formulario y su correspondiente envío. Idealmente el servidor debería rechazar `tools: []` con scope `mcp` (cliente inerte) para que el fallo sea visible en el alta.

### BH-02 — El diálogo de la clave de Marginalia clasifica los errores parseando el mensaje con regex en vez de `error.status`: 401/403 se reportan como «clave inválida»

- **Clasificación:** CONFIRMED_BUG
- **Archivo:líneas:** `crates/amatl-ui/assets/app.js:1489-1506` (`saveMarginaliaKey`, rama `catch`), familia en `crates/amatl-ui/assets/app.js:1010` (`applyProviderSource`) y `crates/amatl-ui/assets/app.js:1897-1905` (`createServerClient`); contraparte correcta en `crates/amatl-ui/assets/client.js:19-29`
- **Síntoma observable:** sin token admin (o con token sin scope admin), «Guardar y verificar» en el diálogo de la clave de Marginalia muestra **«La clave fue rechazada: Marginalia no devolvió resultados con ella. No se guardó.»** (`marginaliaInvalid`) en lugar de los mensajes previstos para 401 (`unauthorized`) y 403 (`marginaliaFailed`) que el propio comentario de la función documenta. El operador concluye erróneamente que su clave privada es mala. Análogamente: el selector de fuentes muestra «No se pudo actualizar las fuentes…» en vez de «token no válido» ante 401 (`applyProviderSource`), y crear un cliente con id duplicado muestra «No se pudo crear el cliente.» en vez de «Ya existe un cliente con ese identificador.» (`clientCreateDuplicate`).
- **Evidencia:**
  - `client.js` construye `error.message = payload?.error?.message` (mensaje de catálogo) y `error.status = response.status`. La cadena `API Error: 401` **solo aparece** en el fallback cuando la respuesta no es JSON.
  - **Sonda en vivo:** los cuerpos de error reales son `{"code":"unauthorized","message":"credential is missing or does not match"}` (401) y `{"code":"scope_denied","message":"credential lacks the required capability"}` (403). Ninguna contiene dígitos de estado → el regex `/API Error: (\d+)/` (`app.js:1493`) nunca casará y cae al `else` = `marginaliaInvalid`. Igual que `.includes("401")` (`app.js:1010`) y `.includes("409")`/`.includes("401")`/`.includes("403")` (`app.js:1900-1903`) que tampoco casan.
- **Cadena síntoma → mecanismo → causa raíz:** mensaje engañoso junto al campo de la clave → la rama `catch` extrae un `status` del *texto* de `error.message` con regex/`includes` en vez de leer `error.status`/`error.code`, que `client.js` ya expone → causa raíz: la clasificación de error se hizo sobre la cadena de reintento `API Error: NNN` de una versión anterior del cliente, y no se actualizó cuando el cliente pasó a parsear el JSON de error del servidor.
- **Impacto:** UX incorrecta y activamente engañosa en el único diálogo donde se teclea un secreto (induce a rotar/descartar una clave válida); mensajes erróneos también en el selector de fuentes y en el alta de clientes (el mensaje específico de duplicado jamás se muestra). Sin impacto en datos ni seguridad.
- **Reproducción:**
  1. Abrir la UI con la configuración de sonda (token admin presente) y un token de operador sin admin, o con el campo de token vacío.
  2. Panel «Insertar clave Marginalia» → pegar cualquier valor → «Guardar y verificar».
  3. El servidor responde 401/403 con JSON de catálogo y el diálogo muestra «La clave fue rechazada…».
- **Corrección mínima sugerida (sin aplicar):** usar `error.status` (y `error.code`) ya calculados por `client.js`: `const status = error && error.status === 403 ? 403 : error && error.status === 401 ? 401 : 0;` y aplicar la misma lógica en `applyProviderSource` y `createServerClient` (o mapear `error.code` con `ERROR_MESSAGE_KEYS`, como ya hacen `reportFailure` y `saveFailureKey`).

### BH-03 — La etiqueta de paginación no acota la página al total: puede mostrar «Página 10 de 1» con resultados vacíos

- **Clasificación:** CONFIRMED_BUG (síntoma menor; alcanzable vía URL compartida/editada a mano)
- **Archivo:líneas:** `crates/amatl-ui/assets/app.js:765-774` (`currentPage` se toma sin acotar), `crates/amatl-ui/assets/app.js:586-590` (`render`: `pageLabel` = `state.page + 1` y habilitado de «Anterior»), `crates/amatl-ui/assets/app.js:854-864` (`restoreFromLocation`: `?page=N` se acepta sin comparar con el total); servidor que hace eco de la página pedida (`GET/POST /search` → `page` tal cual)
- **Síntoma observable:** abrir/buscar con una URL tipo `/?q=mock&page=10` muestra el encabezado de paginación «Página 10 de 1», «No results found.» y el botón «Anterior» habilitado que salta de página en página sin efecto visible hasta llegar a la 1.
- **Evidencia:**
  - **Sonda en vivo:** `POST /search {"q":"mock","page":9,"page_size":10}` → `{"page":9,"page_size":10,"total_results":1,"results":[]}`: el servidor hace eco de la página fuera de rango sin acotarla.
  - `render` usa `state.page + 1` sobre `totalPages = max(1, ceil(total/page_size))` sin clamp; `restoreFromLocation` solo descarta páginas `< 2`, no por encima del total (que aún no se conoce al restaurar).
- **Cadena síntoma → mecanismo → causa raíz:** rótulo contradictorio y navegación sin efecto → el estado `page` del store acepta el valor de la URL y el eco del servidor sin limitarlo a `[0, totalPages-1]` al renderizar → causa raíz: el clamp se aplica solo en los botones «Anterior/Siguiente», no en el origen del valor (`restoreFromLocation` ni el eco del servidor).
- **Impacto:** cosmético/UX: estado de paginación contradictorio enlaces compartidos o editados; sin pérdida de datos. En los flujos con clic el clamp de los botones evita el caso.
- **Reproducción:** UI con `?q=<consulta>&page=10` (con la suite mock basta `q=mock`) y observar el rótulo y el botón «Anterior».
- **Corrección mínima sugerida (sin aplicar):** al renderizar, `const page = Math.min(state.page, state.totalPages - 1)` (o clamp en `store.update` tras conocer `totalResults`), y/o pedir la página eco con `?page=` solo si `< totalPages`.

### BH-04 — `amatl saved show <id>` solo busca entre los 200 documentos más recientes y falla para los antiguos existentes

- **Clasificación:** POTENTIAL_BUG (borde concreto; bajo impacto)
- **Archivo:líneas:** `crates/amatl-cli/src/main.rs:699-707` (`SavedCommand::Show`), orden en `crates/amatl-core/src/storage.rs:1860` (`ORDER BY saved_at DESC, id DESC`)
- **Síntoma observable:** con más de 200 documentos guardados, `amatl saved show <id>` de uno antiguo termina con `no saved document with id <id>` (exit ≠ 0) aunque el documento existe y `saved list`/la base lo contienen.
- **Evidencia:** el comando localiza el documento con `service.saved_documents(200, 0)` y un `.find(|d| d.id == id)` sobre esa ventana; el listado va ordenado por `saved_at DESC`, así que cualquier documento caído de las 200 primeras posiciones es invisible para `Show`. No hay fetch por id en esta superficie.
- **Cadena síntoma → mecanismo → causa raíz:** error falsamente «no encontrado» → la búsqueda por id barre una ventana paginada acotada (200) → causa raíz: implementación de `Show` como «listar página 1 y filtrar» en vez de una lectura por clave primaria (no expuesta por el servicio).
- **Impacto:** CLI operativa: `saved show` falla con un error engañoso en bases con historial largo; deletes por id siguen funcionando (van directos a SQLite).
- **Reproducción:** guardar >200 documentos (o insertarlos), obtener el id del más antiguo y ejecutar `amatl saved show <id>`.
- **Corrección mínima sugerida (sin aplicar):** exponer en `amatl-core` un `saved_document(id)` (SELECT por id) y usarlo en `Show`; mientras tanto, paginar (`offset` decreciente) hasta encontrarlo o declarar el límite en el mensaje de error.

---

## HALLAZGOS CONOCIDOS PREVIOS (2026-13-11): re-verificados

| Hallazgo previo | Estado hoy | Evidencia |
|---|---|---|
| `GET /deep` contradecía la documentación POST-only | **CORREGIDO** | `routes.rs:68` solo registra `post(deep_post)`; teste fijado `deep_is_post_only_and_get_is_method_not_allowed` (`tests.rs:1558+`); sonda en vivo: `GET /deep` → `405` con `allow: POST`. |
| Docs decían «cinco herramientas MCP» siendo seis | **CORREGIDO / consistente** | `MCP_TOOLS` (core) = 6; `tools/list` en vivo devuelve exactamente `answer, deep_search, fetch, providers, search, status`. |
| `browser_e2e` queda verde sin ejecutarse (`return` temprano sin `AMATL_BROWSER_E2E=1`) | **SIN CAMBIOS (documentado)** | `browser_e2e.rs:117-119` y resto de tests: `if !enabled() { return; }`; la suite completa pasó en `0.00 s` sin driver. El header del archivo lo declara «the only legitimate skip», pero seguirá enmascarando regresiones de UI en CI normal. |

---

## NOTAS (no clasificadas como bug)

- **Binario desactualizado en `target/debug`:** el `amatl` preexistente era de antes de los últimos commits (las pruebas de esta auditoría lo detectaron porque no reproducía el comportamiento de los fuentes; `cargo build -p amatl-cli` recompiló). Los tests de `amatl-ui` ya fijan la propiedad «embed == disco», pero un binario viejo sigue sirviendo UI antigua; es riesgo operativo de build, no del código. Clasificación: ENVIRONMENTAL.
- **`OPTIONS` no consume rate limit** (`lib.rs:3036`): deliberado para no romper preflight CORS; la superficie de trabajo es pequeña (no hay handler detrás). No se reporta como bug.
- **Carreras de `store` en `run()`:** una petición abortada/supersedida resta `page`/`pending` de forma silenciosa; hoy no es observable porque el servidor hace eco de `page` y el supersesor re-renderiza. Se deja constancia por si el contrato de `page` cambiara.

---

## QUÉ REVISÉ Y ESTÁ CORRECTO

**Servidor — autenticación y acceso**
- Comparación por digest SHA-256 en tiempo constante (`constant_time_eq`, `lib.rs:3390-3399`) sobre todos los candidatos sin early-exit; digest duplicado entre credenciales falla al arranque/reload (`resolve_clients` → `ServerError::Configuration`), token nunca almacenado en claro.
- Expiración por credencial: fecha inválida falla cerrada (`unexpired_on`, core) y el día se calcula correctamente (algoritmo de Hinnant `civil_from_days` verificado); credencial expirada → `401` con `WWW-Authenticate` y evento `credential_expired` auditado.
- `required_scope` es por (método, ruta) y **fail-closed**: cualquier ruta no declarada cae a `Scope::Admin`; las únicas superficies públicas están listadas explícitamente y solo para GET/HEAD (`/health`, `/ready`, `/metrics`, assets). Verificado en vivo: `/status` con token de solo-search → `403 scope_denied`; sin token → `401`.
- Escalada de scope imposible: el scope se decide sobre la identidad autenticada por el middleware, nunca por cabeceras; PATCH cuenta como mutación en las rutas `/history|/saved`.
- MCP: exactamente seis herramientas; la allowlist sale de la identidad autenticada (extensiones del request, no cabeceras); sonda en vivo: cliente `tools=["search"]` → `search` OK y `fetch` → `scope_denied`; cabecera `mcp-name` que no coincide con el body la rechaza el transporte (`-32020`); sin identidad → `unauthorized`. La lista `tools/list` está acotada al vocabulario de config (herramienta desconocida rechazada en `Config::validate`).
- `/mcp` exige `Scope::Mcp` y `/mcp/*` igual; sin herramienta de ingest en el transporte (decisión fijada por test).

**Servidor — compuerta HTTP**
- Orden de capas correcto: seguridad (exterior) → CORS → límite de cuerpo → rutas; `secured()` aplica cabeceras de seguridad (CSP `self`-only sin `unsafe-*`, `nosniff`, `Referrer-Policy`, `Permissions-Policy`, `X-Frame-Options: DENY`, HSTS solo con TLS) y `X-Request-ID` a **todas** las respuestas, incluidas las de error.
- Host validado (HTTP/1.1 `Host` y HTTP/2 `:authority`, comparación case-insensitive, sin bypass por puerto detectado); Origin validado contra allowlist exacta (ausente = cliente no navegador; `null`/ajena rechazada), CORS limitado a los orígenes efectivos derivados de `allowed_hosts`.
- Tamaño de cabeceras y `Content-Length` sobre el límite → 413/`headers_too_large` auditado; cuerpos chunked sin `Content-Length` quedan cubiertos por `DefaultBodyLimit` en los extractores y por `with_max_request_body_bytes` en MCP.
- Rate limit por IP de conexión (ignora `X-Forwarded-For` a propósito — documentado), ventana acotada (4096 IPs, limpieza periódica), sobrevive a reload (no resetea ventanas), `retry-after` emitido y contabilizado en `/metrics`.
- Timeout de petición con `tokio::time::timeout` sobre el handler; respuestas de error siempre del catálogo (`schema_version` + `error.code` + mensaje sin fugas internas); estados correctos (401/403/404/405/409/413/429/504…).
- Compresión/seguridad del proceso de credenciales de proveedor: clave en `SecretKey` con zeroize, validada con canaria viva antes de aceptarse, nunca escrita en disco/logs ni devuelta; auditoría de credenciales sin el valor.
- Gestión de clientes: alta valida candidato completo antes de escribir, rollback si el reload falla; rotación restaura el digest anterior si el reload falla; PATCH distingue «ausente» vs `null` en `expires_at`; respuestas nunca incluyen secretos salvo el token recién mintado (una sola vez).
- Config mutante: todos los endpoints escritores pasan por `config_write_lock` (sin lost updates), validan candidato completo, y reload se valida contra el **bind efectivo** (impide que un archivo que declara loopback active `no_auth` sobre un listener público).
- `POST /reload` atómico (nada cambia si la validación falla), SIGHUP equivalente y no fatal. `create_backup` con `Idempotency-Key` acotado (128 B, ASCII), TTL 600 s, capacidad 256, reintentos concurrentes serializados sobre la misma celda y VACUUM INTO en tarea desacoplada.
- Paginación de paneles locales acotada (`limit` clamp 1–200) y batch de búsqueda acotado a 5 consultas (`MAX_BATCH_QUERIES`) con resultados independientes por ítem (verificado en vivo con la envoltura `count`/`batches` que coincide con OpenAPI).
- `/metrics` público pero sin nombres de fuentes (solo agregados) — coherente con el comentario de diseño; `/ready` sin detalles internos.
- Pruebas: `required_scope_covers_every_registered_route`, OpenAPI-vs-catálogo de errores, y la batería de scopes/MCP/host/rate limit todo en verde.

**UI (JS/HTML)**
- **Sin XSS:** cero `innerHTML`/`insertAdjacentHTML`/`document.write`/`eval`; toda inserción va por `textContent`/`createElement`/`append`; enlaces solo con `safeHttpUrl` (http/https, sin credenciales en la URL); clases de estado vía allowlist (`PROVIDER_STATUS_CLASSES`); atributos (`title`, `aria-label`, `placeholder`) con setters de propiedad/atributo.
- **i18n:** verificado por script: los catálogos `es`/`en` tienen idénticos 223 claves y todas las claves usadas en `app.js`/`index.html` (incluidas las derivadas `${key}One` de pluralización) existen en ambos. Singular/plural correcto vía `countLabel`/`pluralKey` (también en historial). Cambio de idioma en caliente re-renderiza estado y paneles.
- **Contratos JSON:** `validatePayload` exige `schema_version === "1"` y tipos antes de renderizar; `deepItems`/`evidenceFragments` acotan tamaños y verifican linaje (`provenance_id`, `document_id`, igualdad de URLs); verificación criptográfica de fragmentos con rangos validados antes de decodificar.
- **Estado/carreras:** la vista comprometida no se destruye mientras hay petición en vuelo (`pending` suprime el render y restaura tras fallo/cancelación); supersesión detectada por identidad de `AbortController`; abort no borra resultados; foco preservado en los toggles (`updateSilent`).
- **Paginación server-side real:** `page`/`page_size` viajan al servidor y `totalPages` se calcula de `total_results`; sin ventaneo local (`state.items.slice` prohibido por test).
- **CSRF:** no hay cookies de sesión; el token va en `Authorization` (memoria, borrado en `pagehide`), los POST son JSON con `Content-Type` y el Origin se valida en el servidor.
- Formularios accesibles (labels, `aria-describedby`, `role="status"/"alert"`), `hidden` con override CSS `!important` fijado por test, y CSP coherente entre cabecera y `<meta>` (éste sin `frame-ancestors`, correctamente).

**CLI**
- Códigos de salida: error → exit 1 con `error_code=<catálogo>` en stderr (separado del JSON en stdout); argumentos inválidos → exit 2 (clap); fallo de `search` con contrato `Failure` emite los códigos del error compuesto antes de salir.
- `--json` serializa respuestas del núcleo tal cual (misma `schema_version`), el resto de comandos imprime formas documentadas; sin `unwrap`/`expect` sobre rutas o entradas de usuario en los flujos de comando (los `unwrap` presentes son de tests o de constantes estáticas).
- Redacción de logs: JSON con redacción de campos sensibles (`token`, `key`, `authorization`, …) y filtro de targets; verificado por tests unitarios.

**Pruebas de integración**
- `soak.rs` correctamente marcadas `#[ignore]` con instrucciones; `browser_e2e` documentado arriba (skip condicionado).