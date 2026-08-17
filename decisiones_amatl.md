# Decisiones de arquitectura — AMATL

Registro histórico ADR. `plan_amatl.md` y `fase_a_contratos.md` son las fuentes
normativas activas; este archivo explica decisiones ya materializadas y no las
reemplaza. Fecha base trazable: commit `51c6d34`, 2026-08-12.

## ADR-001 — Contratos rectores protegidos

- **Fecha:** 2026-08-12
- **Estado:** Aceptada
- **Contexto:** La implementación depende de invariantes y contratos cerrados;
  cambios incidentales provocarían deriva entre fases.
- **Decisión:** `plan_amatl.md` y `fase_a_contratos.md` no se modifican durante
  desarrollo ordinario. Un cambio contractual exige propuesta dedicada,
  impacto, migración, fixtures y aprobación explícita.
- **Consecuencias:** Código, calibración y documentación evolucionan fuera de
  ambos archivos; el gate debe detectar cambios accidentales.
- **Alternativas descartadas:** tratar cada documento como notas editables o
  sustituirlos por `CONTINUIDAD.md`.

## ADR-002 — Licencia dual MIT o Apache-2.0

- **Fecha:** 2026-08-12
- **Estado:** Aceptada
- **Contexto:** El workspace ya declara `MIT OR Apache-2.0` y la distribución
  necesita textos completos.
- **Decisión:** Cada usuario elige MIT o Apache-2.0; contribuciones intencionales
  se reciben bajo los mismos términos salvo declaración explícita.
- **Consecuencias:** Se distribuyen `LICENSE-MIT` y `LICENSE-APACHE`; dependencias
  siguen sujetas a sus propias licencias y `cargo deny`.
- **Alternativas descartadas:** licencia única o términos propietarios, porque
  contradicen el manifiesto vigente.
- **Trazabilidad:** `Cargo.toml:6`; `LICENSE-MIT`; `LICENSE-APACHE`.

## ADR-003 — rustls para HTTP/TLS

- **Fecha:** 2026-08-12
- **Estado:** Aceptada
- **Contexto:** La distribución principal busca binario Linux musl estático y
  OpenSSL añadiría una dependencia nativa evitable.
- **Decisión:** Reqwest desactiva features por defecto y usa `rustls-tls`; Axum
  Server usa `tls-rustls`.
- **Consecuencias:** No hay backend OpenSSL en el diseño principal; cualquier
  desviación debe justificarse y probar su empaquetado.
- **Alternativas descartadas:** OpenSSL del sistema y TLS terminado únicamente
  por proxy, porque el bind remoto exige TLS configurado en AMATL.
- **Trazabilidad:** `Cargo.toml:16-19`; `amatl-server/src/lib.rs:168-190`.

## ADR-004 — SQLite fuera de correctness

- **Fecha:** 2026-08-12
- **Estado:** Aceptada
- **Contexto:** Caché y telemetría aportan rendimiento/adaptación, pero no deben
  convertir una falla local de storage en una falla de Search.
- **Decisión:** Persistencia y cachés están desactivadas por defecto; abrir,
  leer, escribir o restaurar SQLite es tolerante a fallos.
- **Consecuencias:** Sin SQLite se pierde estado/cache y el routing reinicia en
  Bootstrap, pero Search continúa. WAL, NORMAL, timeout 5 s y pool 4 acotan
  concurrencia cuando se habilita.
- **Alternativas descartadas:** hacer obligatoria la base o cachear la salida
  final de Search.
- **Trazabilidad:** `config.rs:382-411`; `service.rs:101-112`; `storage.rs:58-118`.

## ADR-005 — DuckDuckGo HTML bloqueado por gobernanza

- **Fecha:** 2026-08-12
- **Estado:** Aceptada
- **Contexto:** Un adapter HTML `best_effort` no implica autorización para
  scraping ni certeza sobre ToS/coste.
- **Decisión:** DuckDuckGo HTML siempre declara
  `provider_pending_explicit_approval` y no ejecuta red hasta una revisión
  verificable y ficha completa.
- **Consecuencias:** Incluir el nombre en `providers.enabled` no lo activa; su
  ausencia nunca rompe el estado global si hay resultados útiles.
- **Alternativas descartadas:** habilitación implícita, scraping oportunista o
  asumir permiso por acceso público.
- **Trazabilidad:** `providers/duckduckgo.rs:8-59`; `service.rs:298,349`.

## ADR-006 — Trafilatura como capability opcional

- **Fecha:** 2026-08-12
- **Estado:** Aceptada
- **Contexto:** La extracción editorial mejora Deep, pero el binario base no
  debe depender de Python ni de un extractor irreversible.
- **Decisión:** `Extractor` es la frontera reemplazable; Trafilatura se ejecuta
  como proceso externo acotado por stdin/stdout, tiempo y bytes.
- **Consecuencias:** Si falta o falla, Deep conserva un Document superficial; el
  riesgo del proceso externo permanece explícito.
- **Alternativas descartadas:** enlazar Python al core o ejecutar Trafilatura en
  Search.
- **Trazabilidad:** `extract.rs:36-167`; `deep.rs`; `tests/deep_phase5.rs:271-285`.

## ADR-007 — Bearer local obligatorio en HTTP y MCP

- **Fecha:** 2026-08-12
- **Estado:** Aceptada
- **Contexto:** API y MCP pueden disparar providers y fetches; incluso en local
  necesitan una frontera explícita.
- **Decisión:** Rutas protegidas usan un token de entorno de al menos 32 bytes.
  `no_auth` sólo existe para desarrollo en loopback; bind remoto requiere token
  y TLS.
- **Consecuencias:** La UI solicita el token manualmente y no crea cookies ni
  sesión. Es un secreto compartido, sin roles ni multi-tenancy.
- **Alternativas descartadas:** servidor sin autenticación por defecto, token en
  TOML o sesión basada en cookie.
- **Trazabilidad:** `config.rs:487-503,616-650`;
  `amatl-server/src/lib.rs:93-108,348-427`.

## ADR-008 — `AmatlService` como núcleo único

- **Fecha:** 2026-08-12
- **Estado:** Aceptada
- **Contexto:** CLI, API, MCP y UI deben producir el mismo contrato y no divergir
  en routing, Budget o seguridad de Deep.
- **Decisión:** Las superficies delegan `search`, `deep` y provider summaries a
  `AmatlService`; sólo adaptan transporte y presentación. MCP selecciona límites
  más estrictos mediante `ServiceSurface`.
- **Consecuencias:** Cambios funcionales viven en core y se prueban una vez; los
  tests de cada superficie validan serialización y exposición.
- **Alternativas descartadas:** orquestadores independientes o lógica de negocio
  en handlers/UI.
- **Trazabilidad:** `service.rs:14-58,94-382`; `amatl-cli/src/main.rs:159-219`;
  `amatl-server/src/lib.rs:214-293`; `amatl-server/src/mcp.rs:36-98`.

## ADR-009 — Políticas calibrables, contratos versionados

- **Fecha:** 2026-08-12
- **Estado:** Aceptada
- **Contexto:** Ranking, Diversity, búsqueda progresiva y Gap necesitan ajuste
  empírico sin romper JSON.
- **Decisión:** La forma/rangos se fijan en políticas con versión (`v1`/`v2`),
  mientras valores válidos se calibran sin cambiar `schema_version`. Ranking v2
  sólo se aplica si supera su corpus humano versionado.
- **Consecuencias:** Una ruptura de datos incrementa `schema_version`; una
  calibración compatible no. SemVer y migración SQLite son ejes separados.
- **Alternativas descartadas:** constantes opacas, ranking no explicable o
  promover Ranking v2 sin benchmark.
- **Trazabilidad:** `model.rs:7`; `ranking.rs`; `diversity.rs`; `progressive.rs`;
  `ranking_v2.rs:11-128`; corpus `ranking_v2_corpus.json`.

## ADR-010 — Retiro de DuckDuckGo HTML; Marginalia pasa de scaffold a adapter real

- **Fecha:** 2026-08-15
- **Estado:** Aceptada
- **Contexto:** ADR-005 bloqueaba DuckDuckGo HTML fail-closed a la espera de
  revisión, pero DuckDuckGo no ofrece API de búsqueda web (sólo Instant
  Answer, que no devuelve resultados web); el adapter era un stub de 73
  líneas sin endpoint, cliente HTTP ni parseo que nunca podría aprobar
  gobernanza. Mantenerlo listado era una fuente "en progreso" fantasma.
  Marginalia, por otro lado, sí tiene API de búsqueda oficial y era el
  scaffold señalado como siguiente paso en `CONTINUIDAD.md`.
- **Decisión:** Se retira `duckduckgo_html` del código (`providers/duckduckgo.rs`
  eliminado), del `ProviderRegistry`, de `config.rs` y de la documentación de
  gobernanza — no queda como adapter apagado, deja de existir. Se implementa
  `search()` real de Marginalia contra `api2.marginalia-search.com` (el
  endpoint `api.marginalia.nu` original está deprecado), con el header
  `API-Key`, traducción de `site:` y manejo tipado de errores/rate limit. El
  router (`AdaptiveRouter`) añade una penalización proporcional a
  `estimated_cost` para que una fuente de pago no ocupe la primera ronda sólo
  por un mal día de latencia/salud de una fuente gratuita.
- **Consecuencias:** ADR-005 queda históricamente correcta para su fecha, pero
  superada: ya no aplica porque el sujeto que bloqueaba no existe. La ficha de
  Marginalia pasa a "aprobable" en `docs/gobernanza-providers.md`, pendiente
  sólo de `reviewer`/`reviewed_at`/`approval_status` (decisión del propietario,
  no de código). SearXNG sigue siendo la única fuente sin credencial.
- **Alternativas descartadas:** implementar el stub de DuckDuckGo HTML (sin API
  de búsqueda real, exige scraping sin ToS verificable — contradice la puerta
  de gobernanza); mantener el criterio de routing ciego a coste.
- **Trazabilidad:** `providers/marginalia.rs`; `router.rs` (penalización por
  `estimated_cost`); `providers/registry.rs`; `config.rs::builtin_provider_records`;
  `docs/gobernanza-providers.md`.

## ADR-011 — Síntesis de respuesta opcional ("Resumen con IA") sobre resultados de Search

- **Fecha:** 2026-08-16
- **Estado:** Aceptada
- **Contexto:** `plan_amatl.md` y `fase_a_contratos.md` prohíben introducir un
  LLM obligatorio, pero no prohíben uno opcional, apagado por defecto y con
  decisión explícita — la misma cláusula que ya exige `data_policy` para
  cualquier salida de red nueva. El propietario pidió sintetizar los
  resultados de Search en una respuesta citada, sujeta a dos condiciones no
  negociables planteadas desde el inicio: la credencial nunca toca el
  navegador ni el archivo de configuración (sólo variable de entorno), y la
  respuesta debe ser verificable contra fuentes reales, no texto generado
  libremente.
- **Decisión:** Nuevo módulo `amatl-core/src/answer.rs`, gateado por dos
  interruptores independientes: `data_policy.inference = "remote_explicit"`
  (mismo gate que el backend de embeddings remoto) y `answer.enabled` en
  `[answer]`. El modelo sólo ve los resultados que Search ya obtuvo —título,
  URL, snippet acotado por `max_source_chars`— nunca contenido no obtenido
  por AMATL. Cada cita `[n]` se valida contra los índices de fuente reales
  tras la llamada (`extract_citations`); una cita a una fuente que no existe
  se elimina del texto visible, no sólo del conteo (`strip_invalid_citations`,
  UTF-8-safe); una respuesta sin ninguna cita válida se rechaza como
  `AnswerError::Ungrounded` en vez de mostrarse. Expuesto por igual en HTTP
  (`POST /answer`), MCP (`answer`) y UI (botón `Resumen con IA`, siempre
  visible, visualmente deshabilitado cuando no está disponible). Un
  interruptor admin-scoped (`POST /answer/enabled`) permite activarlo/
  desactivarlo desde la propia UI: valida la configuración candidata completa
  antes de escribir nada, y escribe sólo la clave `answer.enabled` en
  `amatl.toml` con `toml_edit`, preservando comentarios y el resto del
  archivo. `AnswerStatus` separa `enabled`/`configured`/`available` como tres
  campos independientes a propósito, para que apagar la función no oculte el
  propio panel de configuración que permite volver a encenderla.
- **Consecuencias:** Primer backend de inferencia remota realmente activo en
  un despliegue de operador (antes sólo existía el gate de `data_policy`, sin
  código que lo usara). Amplía la superficie de amenaza con una ruta de
  egress gobernada hacia un tercero (hoy DeepInfra) — documentado en
  `docs/security/threat-model.md` bajo `Core → inference (answer)`, incluida
  la matización de que el grounding acota qué puede citarse, no qué puede
  decir el modelo en prosa libre ante una fuente hostil con inyección de
  instrucciones. `README.md` mantiene "no es un chatbot... ni sistema
  dependiente de LLM" porque sigue siendo cierto en sentido estricto: es una
  síntesis de un solo turno sobre resultados ya obtenidos, no una
  conversación con memoria, y permanece apagada hasta decisión explícita del
  operador. Un chat conversacional multi-turno se evaluó y se descartó
  explícitamente por ahora (ver «Pendientes» más abajo): cambiaría esa
  invariante y exige su propia decisión.
- **Alternativas descartadas:** dejar la síntesis sin verificación de citas
  (harían al texto generado indistinguible de una alucinación con apariencia
  de fuente real); atar `configured` a `enabled` en el estado expuesto
  (bug real detectado y corregido antes de enviar: apagar la función ocultaba
  el propio interruptor para volver a encenderla); permitir memoria
  multi-turno o persistencia de conversación en esta misma entrega (mayor
  superficie de retención de datos y de costo por sesión sin límite
  equivalente al `Budget` de Search — evaluado aparte, sin implementar).
- **Trazabilidad:** `answer.rs`; `config.rs::set_answer_enabled`;
  `amatl-server/src/lib.rs::{answer_post,answer_toggle}`;
  `service.rs::AnswerStatus`; `docs/resumen-con-ia.md`;
  `docs/security/threat-model.md`; `docs/api/openapi.yaml`.
