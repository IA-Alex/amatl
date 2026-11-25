# Bug hunt adversarial — amatl (core / server / cli / ui)

- **Fecha:** 2026-11-24
- **Alcance:** `crates/amatl-core/src/`, `crates/amatl-server/src/`, `crates/amatl-cli/src/main.rs`, `crates/amatl-ui/` (superficie de producto; se leyeron los módulos de mayor riesgo: `normalize`, `security`, `fetch`, `deep`, `ingest`, `extract`, `render`, `child_process`, `storage`, `service`, `ranking`, `evidence`, `optics`, `canonical`, `robots`, `config`, y en el servidor `lib.rs` (auth/middleware), `mcp.rs`, `routes.rs`).
- **Metodología:** skill `bug-hunter` (pipeline adversarial `Hunter → Skeptic → Referee`, política *fail-closed*, trazabilidad *source-to-sink*). Lectura simbólica con Serena + reproducción independiente mediante un crate scratch en `/tmp/user/1000/kilo/bh-repro` que depende de `amatl-core` por path.
- **HEAD auditado:** `e11c592e`.
- **Base de pruebas:** `cargo test -p amatl-core --lib` → **385 passed; 0 failed**. La línea base es sana: los fallos encontrados **no están cubiertos** por los tests existentes.
- **Modo:** solo lectura. No se modificó ningún archivo del repositorio.

**Resumen:** 3 CONFIRMED (1 MEDIA, 2 BAJA). 0 CRÍTICAS/ALTAS nuevas. Se re-verifican hallazgos abiertos del 2026-11-18.

| ID | Veredicto | Severidad | Ubicación | Categoría | Resumen |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `BH-001` | CONFIRMED | MEDIA | `normalize.rs:133-173` | Seguridad / Contrato | Entidades HTML se decodifican **después** de validar chars de control → inyecta ESC/NUL/U+FFFD en títulos/snippets |
| `BH-002` | CONFIRMED | BAJA | `ingest.rs:547-567` | Lógica / Parser | Off-by-one en el escape octal PDF: se pierde el byte siguiente a un `\ddd` |
| `BH-003` | CONFIRMED | BAJA | `cli/main.rs:706-713` | Lógica / Regresión | `saved show --id N` falla para ids fuera de los 200 más recientes (regresión de B3, 2026-11-18) |

---

## BH-001 — [MEDIA] Entidades HTML decodificadas después de la validación permiten inyectar caracteres de control

- **Ubicación:** `crates/amatl-core/src/normalize.rs:133-146` (`clean_text`) y `normalize.rs:148-173` (`decode_html_entities`).
- **Ruta de fallo (source → sink):**
  1. `ProviderItem.title`/`snippet`/`author`/`metadata` provienen de la respuesta de un proveedor (HTTP de terceros → **entrada controlable** por un proveedor comprometido, MITM o resultado inyectado en el metabuscador).
  2. `clean_field` → `clean_text` valida el texto **crudo**: rechaza `\0`, `U+FFFD` y cualquier `char::is_control() && !is_whitespace()`.
  3. **Después** de validar, `clean_text` llama a `decode_html_entities`, que convierte `&#27;` → `ESC`, `&#0;` → `NUL`, `&#xfffd;` → `U+FFFD` mediante `char::from_u32` (`normalize.rs:167-170`).
  4. El texto ya decodificado **no se re-valida** ni se registra degradación.
  5. Sink: el CLI imprime `result.title` sin escapar (`cli/main.rs:1239`, `print_search`) y la misma cadena llega al JSON (`serde_json`) y a la UI.
- **Prueba de concepto (ejecutada).** Crate scratch contra `amatl-core`, `normalize` con `title = "SAFE&#27;&#91;&#51;&#49;&#109;PWNED"` (ESC `[31m`) y `snippet = "cero&#0;fin"` (NUL):

  ```text
  [BH-001] degradations = []
  [BH-001] title bytes   = [83, 65, 70, 69, 27, 91, 51, 49, 109, 80, 87, 78, 69, 68]
  [BH-001] ESC in title  = true
  [BH-001] NUL in snippet= true
  [BH-001] json title    = "SAFE\u001b[31mPWNED"
  [BH-001v] title with U+FFFD = true   (vía &#xfffd;)
  [BH-001sanity] raw ESC title = None  (el ESC crudo SÍ se rechaza correctamente)
  ```

  La línea `sanity` prueba que la validación corre primero: un ESC literal se descarta, pero `&#27;` lo reintroduce. `degradations` queda vacío, contradiciendo el contrato documentado de `clean_text`.
- **Impacto.** Inyección de secuencias ANSI/CSI (terminal escape injection) en la terminal del operador a través de resultados de búsqueda de proveedores hostiles; NUL en campos que aguas abajo se tratan como C-strings o se vuelcan a herramientas externas; violación directa del contrato de `clean_text` ("rechaza chars de control y U+FFFD"). Vector: un proveedor comprometido o una respuesta inyectada basta. Alcance MEDIA (no RCE ni exfiltración directa, pero afecta a la integridad de la salida y a la superficie de la terminal).
- **Nota del Skeptic/Referee.** El Skeptic intentó refutar por "ya hay validación": falla, porque la validación precede a la decodificación (demostrado con el caso `sanity`). No hay sanitización posterior en CLI/UI/JSON. **Sobrevive.**
- **Variantes.** La decodificación de entidades solo existe en `normalize.rs`; no se encontró un segundo decodificador de entidades equivalente en `extract.rs`/`render.rs`/`text.rs`. Pero **esta misma "decodificación que reintroduce datos no validados" reaparece en BH-002** (bytes 0x00-0xFF del `parse_hex_string`/`parse_literal_string` de PDF entran como chars crudos en `document.content`).

## BH-002 — [BAJA] Off-by-one en el escape octal del parser PDF en Rust

- **Ubicación:** `crates/amatl-core/src/ingest.rs:547-567` (`parse_literal_string`). Concretamente, el avance del índice tras un escape octal: `i += count;` (563) seguido de `i += 2;` (567).
- **Ruta de fallo:**
  1. Un documento PDF descargado o local (bytes no confiables) se ingiere; bajo política `isolated` (o si `pdftotext` no está disponible) se usa el extractor puro-Rust `extract_pdf_rust`.
  2. `parse_literal_string` procesa un string PDF con un escape octal `\ddd` (`bytes[i]=b'\\'`, `next` en `0..=7`).
  3. El bucle consume hasta 3 dígitos octales → `count`. El avance correcto sería `i += 1 + count`; el código hace `i += count` y luego cae al `i += 2` común del camino no-octal → avanza **un byte de más**, saltándose el carácter que sigue al `\ddd`.
- **Prueba de concepto (ejecutada).** Scratch con `LocalIngestor` (política isolated, sin egress) sobre un PDF cuyo stream es `(A\101BCD) Tj` (`\101` = `A`):

  ```text
  [BH-002] extracted content = Some("AACD")
  [BH-002] expected (correct)  = "AABCD"
  [BH-002] lossless = false
  ```

  Esperado `AABCD`, obtenido `AACD`: se perdió la `B` inmediatamente posterior al escape octal.
- **Impacto.** Corrupción silenciosa (pérdida de uno) de texto extraído de PDFs que usan escapes octales (habitual en PDFs generados por LaTeX/impresoras virtuales y en secuencias CID). Bajo: el texto sigue siendo utilizable; el fallback solo aplica cuando el extractor externo no corre. No hay panic ni corrupción de memoria.
- **Nota del Skeptic/Referee.** El Skeptic buscó refutación: el test existente `pdf_rust_extractor_handles_escapes_and_nested_parens` (`ingest.rs:1089`) cubre `\(`, `\)`, `\n` pero **no** `\ddd`, por eso el defecto no se detecta. La reproducción es determinista. **Sobrevive.**
- **Variantes.** Se auditó el resto de la aritmética de índices del parser (`collect_pdf_text`, `parse_hex_string`, `parse_text_array`, `find_stream_keyword`, `find_sub`) y del resto del crate (`canonical.rs`, `robots.rs`, `evidence.rs`): no hay un segundo off-by-one equivalente. **Ninguna.**

## BH-003 — [BAJA] `saved show --id N` falla para ids fuera de los 200 documentos más recientes (regresión de B3)

- **Ubicación:** `crates/amatl-cli/src/main.rs:706-713` (`SavedCommand::Show`), apoyado en `service.rs:1320-1327` (`saved_documents`) → `storage.rs:1870-1903` (`saved_document_list`, orden `saved_at DESC, id DESC`, sin lookup por id).
- **Ruta de fallo:** `saved show --id N` implementa un "get por id" escaneando solo la ventana `saved_documents(200, 0)`; no existe `saved_document_get_by_id`. Con >200 documentos guardados, un id válido y antiguo no aparece en la ventana → el CLI imprime `no saved document with id N`, mientras `saved delete --id N` sí lo encuentra (usa `saved_document_delete(id)` real, `storage.rs:1908`).
- **Impacto.** Falso negativo en una superficie de operación; el listado sin `--json` imprime ids, invitando al flujo `show`. No hay corrupción de datos.
- **Nota del Skeptic/Referee.** Código idéntico al reportado el 2026-11-18 (B3) y **no reparado**; la lógica es inequívoca y ya fue reproducida entonces contra `SqliteStorage` real. **Sobrevive** (confirmado por inspección; la reproducción previa sigue siendo válida al no haber cambiado el código).
- **Variantes.** No hay otras rutas que emulen "get por id" con ventana fija.

---

## Re-verificación de hallazgos abiertos del 2026-11-18

- **B1 (normalize, control chars) → sigue ABIERTO, ahora reportado como `BH-001`** (reproducido hoy).
- **B2 (fragmentos #a/#b/#c duplicados en Deep) → ABIERTO.** `deep.rs:440` sigue insertando en `visited` la URL **con** fragmento (`link.as_str()`), mientras `child_result` (`deep.rs:662-663`) hace `url.set_fragment(None)` después de encolar. La clave de dedupe y la forma canónica siguen discrepando. No lo elevé a hallazgo nuevo por ser duplicado del ya documentado; **recomendación: reparar en el mismo lote que BH-001…BH-003**.
- **B3 (`saved show`) → sigue ABIERTO**, ahora `BH-003`.
- **B4 (claves desconocidas en `amatl.toml` ignoradas en silencio / fail-open en `data_policy`) → ABIERTO.** `Config::from_toml` (`config/mod.rs:272-290`) sigue usando `toml::from_str` sin `deny_unknown_fields` ni pasada de "claves desconocidas"; `validate()` no las detecta. Sigue siendo un riesgo de endurecimiento (un typo en `[data_policy]` deja `Standard`/`Governed` con egress permitido sin aviso). No re-reproducido en esta sesión (mecanismo ya demostrado el 2026-11-18); clasificado **HARDENING** de nuevo.
- **B5/B6/B7 (redirects no contabilizados en fallo; casts `as i64` sin cota en TTL; `fetch_timeout` de Deep con `now` obsoleto) → ABIERTOS.** Confirmado por lectura: `deep.rs:287` sigue cobrando redirects solo en el camino `Ok`; `storage.rs` mantiene los casts `as i64`; `deep.rs:231-234` sigue calculando `fetch_timeout` con el `now` de `deep.rs:176` (anterior al `sleep` de Crawl-delay). Sin cambios que los cierren.

## Qué revisé y está correcto (cobertura verificada)

- **`security.rs` (SSRF):** esquemas, credenciales embebidas, hosts internos/single-label, IPv4 (privada, loopback, link-local, broadcast, documentación, 0/8, 240/4, CGNAT, 192.0.0/24, 198.18/15), IPv6 (mapeada y compatible IPv4, ULA, link-local, 2001:db8, NAT64, 6to4). Sólido.
- **`fetch.rs`:** DNS validado y cliente pinneado a las direcciones validadas (sin TOCTOU), redirects re-validados, cabeceras por allowlist, límite de bytes en streaming, `no_proxy`/`referer(false)`, Debug redactado, recuperación de mutex con evento de seguridad.
- **`child_process.rs`:** allowlist de entorno sin secretos (con test que lo garantiza), scrub en **todos** los sitios de spawn (`extract.rs`, `ingest.rs`, `render.rs`), reintento acotado solo de `ETXTBSY`.
- **`extract.rs`/`ingest.rs`:** `stdout.take(max+1)` con rechazo `OutputLimit`, timeouts acotados, `kill_on_drop`, `pdftotext` con args fijos (`-`,`-`) y env scrubbed; únicos defectos: BH-002.
- **`render.rs`:** sandbox con env mínimo explícito, `ScratchDirectory` creado con `create_dir` (falla si existe → fail-closed) y modo 0700, deadline con backstop, cap de concurrencia por semáforo.
- **Servidor (`lib.rs`):** middleware de seguridad con límites de headers/body, validación de `Host`/`Origin`, rate limit por IP con evicción acotada, autenticación en tiempo constante sobre digests SHA-256, `required_scope` **fail-closed** por método+ruta, exención pública solo para GET/HEAD de assets/probes, token one-time con guarda en la UI. Sin bypass encontrado.
- **`mcp.rs`:** autorización por allowlist de herramientas desde la identidad del middleware (nunca de un header del cliente); "sin identidad → denegar".
- **`storage.rs`:** SQL 100% parametrizado; el único `format!` es un patrón `LIKE` con escapado correcto (backslash primero) y `ESCAPE '\'`.
- **`dedupe.rs`/`ranking.rs`/`evidence.rs`/`optics.rs`/`canonical.rs`/`robots.rs`:** grupos no vacíos garantizados por `into_values()`, división por `active_provider_count-1` protegida por `<= 1`, slices derivados de `char_indices`/`floor_char_boundary`, bucles `len()-1` nunca evaluados con colección vacía, `path.get(..)` seguro.
- **Desde-: `duration_since` con `UNIX_EPOCH` usa `unwrap_or_default`; sin restas que puedan paniquear en producción.**

## Contrato JSON para remediación (handoff a software-repair-engineer)

```json
{
  "audit_status": "completed",
  "head": "e11c592e",
  "findings": [
    {
      "id": "BH-001",
      "verdict": "CONFIRMED",
      "severity": "MEDIUM",
      "file": "crates/amatl-core/src/normalize.rs",
      "lines": "133-173",
      "category": "sanitization_order",
      "root_cause": "clean_text valida caracteres de control del texto crudo y luego llama a decode_html_entities, que reintroduce ESC/NUL/U+FFFD sin re-validar.",
      "trigger": "ProviderItem.title/snippet con '&#27;', '&#0;' o '&#xfffd;' (respuesta de proveedor hostil).",
      "variants": ["crates/amatl-core/src/ingest.rs:601,609 (parse_hex_string emite chars de control crudos en document.content)"]
    },
    {
      "id": "BH-002",
      "verdict": "CONFIRMED",
      "severity": "LOW",
      "file": "crates/amatl-core/src/ingest.rs",
      "lines": "547-567",
      "category": "parser_off_by_one",
      "root_cause": "Tras un escape octal \\ddd, parse_literal_string avanza 'count' mas el '+2' comun del camino no-octal, saltando el byte siguiente.",
      "trigger": "PDF con stream '(A\\101BCD) Tj' bajo extractor puro-Rust: se extrae \"AACD\" en vez de \"AABCD\".",
      "variants": []
    },
    {
      "id": "BH-003",
      "verdict": "CONFIRMED",
      "severity": "LOW",
      "file": "crates/amatl-cli/src/main.rs",
      "lines": "706-713",
      "category": "false_negative_lookup",
      "root_cause": "saved show emula get-por-id escaneando solo saved_documents(200,0); no existe lookup por id en storage.",
      "trigger": ">200 documentos guardados; 'saved show --id <antiguo>' -> 'no saved document with id N' pese a existir.",
      "variants": []
    }
  ],
  "hardening_recommended": [
    { "id": "HB-B4", "file": "crates/amatl-core/src/config/mod.rs", "note": "from_toml sin deny_unknown_fields: typo en [data_policy] deja egress abierto en silencio (fail-open de diseno)." },
    { "id": "HB-B5", "file": "crates/amatl-core/src/deep.rs", "note": "consume_fetch cobra redirects solo en el camino Ok (deep.rs:287): el presupuesto global de redirects se excede hasta F*R." },
    { "id": "HB-B2", "file": "crates/amatl-core/src/deep.rs", "note": "visited.insert usa la URL con fragmento (deep.rs:440) mientras child_result la limpia despues (deep.rs:663): duplicados por #a/#b/#c." },
    { "id": "HB-B6", "file": "crates/amatl-core/src/storage.rs", "note": "casts u64 as i64 sin cota en aritmetica de TTL; validate_cache sin cota superior." },
    { "id": "HB-B7", "file": "crates/amatl-core/src/deep.rs", "note": "fetch_timeout usa un 'now' anterior al sleep de Crawl-delay (deep.rs:176 vs 231): Deep puede exceder su deadline hasta 5 s." }
  ],
  "manual_review_required": []
}
```

## Puntos ciegos / limitaciones

- No se ejecutó una reproducción con servidor HTTP real para confirmar B5/HB-B5 (redirects en fallo); el mecanismo es inequívoco por lectura de código pero la métrica exacta `F×R` no se midió en esta sesión.
- No se auditaron los crates excluidos del workspace (`experiments/*`) ni los binarios externos (`pdftotext`, `trafilatura`, Chromium), solo su invocación desde Rust.
- Cobertura de `amatl-server/src/tests.rs` (6124 líneas) fue por muestreo del código de producción que ejercita, no test por test.
