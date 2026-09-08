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

## ADR-012 — Adquisición confirmatoria con compuerta previa de novedad y diversidad

- **Fecha:** 2026-09-04
- **Estado:** Aceptada
- **Contexto:** V5 terminó `COMPLETE_INCONCLUSIVE` con SearXNG, sin fallos HTTP:
  2,172/2,172 requests fueron exitosos. El universo congelado se agotó antes de
  474 resultados válidos por brazo. Esta decisión cierra el análisis post-V5;
  no autoriza V6, no reabre V5 y no modifica ground truths ni resultados V1–V5.
- **Evidencia y funnel:** El ledger V5 (`docs/evaluation/independent-relevance/v5/execution/`)
  contiene 2,172 queries y 3,870 filas raw. La reconciliación es:
  `3,870 - 1,354 DUPLICATE_CURRENT - (911 OVERLAP_V1 + 167 OVERLAP_V3 +
  811 OVERLAP_V4) = 627 VALID_RESULTS`; `REJECTED_TOTAL=3,243` y
  `627 + 3,243 = 3,870`. Por tanto, `RAW_RESULTS_PER_QUERY=1.7818`,
  `VALID_RESULTS_PER_QUERY=0.2887`, `VALID_RATE_RAW=16.20%` y
  `REJECTION_RATE_RAW=83.80%`. Participación de rechazo: duplicación 41.75%,
  V1 28.09%, V3 5.15% y V4 25.01%. `CANONICAL_RESULTS=2,516` después de
  deduplicación actual; `FINAL_ACCEPTED_RESULTS=627`.
- **Saturación y duplicación:** El V5 raw tiene 916 URLs canónicas únicas
  (23.67% de 3,870); los 1,354 duplicados proceden de
  `CROSS_QUERY_DUPLICATION=1,354`: 1,320 cruces entre pares y 34 repeticiones
  dentro del mismo par; 339 de esas filas cruzan brazos. No se observó
  duplicación dentro de una misma query.
  Hay 322 URLs distintas entre las filas rechazadas específicamente por
  `DUPLICATE_CURRENT`. Las más repetidas son
  `vixra.org/astro` (46), `millermicro.com` (44), `unabomber.neocities.org`
  (34), `greenmagi.com` (34) y `billdietrich.me` (39 en duplicados).
  Los cuatro motivos históricos suman 1,889 filas (48.81% de raw); los
  conteos por versión son V1=911 (23.54%), V2=0 (0%), V3=167 (4.32%) y
  V4=811 (20.96%). Esto demuestra saturación histórica severa del espacio
  recuperado bajo el universo V5. La reconstrucción de los conjuntos
  canónicos históricos usados por el capturador da V1=555, V2=10, V3=82,
  V4=382 y unión histórica=923 URLs; las intersecciones por versión son
  V1∩V2=9, V1∩V3=23, V1∩V4=62, V2∩V3=1, V2∩V4=2 y V3∩V4=18. V5 comparte
  289 de sus 916 URLs únicas con esa unión (31.55%); a nivel de filas raw,
  la contaminación histórica observada es 48.81%. Las 627 URLs aceptadas
  son nuevas respecto de esa unión.
- **Diversidad efectiva:** Las 2,172 cadenas son únicas literalmente y tras
  normalización de espacios/case (`QUERY_UNIQUENESS=100%`), pero representan
  1,086 pares/temas, 31 conceptos base seleccionados, 36 contextos y 8
  formas lingüísticas repetidas por brazo. La similitud Jaccard media de los
  conjuntos de resultados de los pares fue 27.18%; 844 queries tuvieron un
  conjunto idéntico a uno previo y el mayor grupo idéntico tuvo 629 filas.
  La diversidad nominal, por ello, sobrestima la diversidad documental.
  `DOMAIN_CONCENTRATION` se reporta como alta cualitativamente cuantificable:
  los 15 dominios principales concentran 1,002/3,870 filas (25.89%); no se
  usa una cifra de concentración de dominios únicos porque el ledger no la
  define como métrica normativa.
- **Treatment vs control:** Treatment produjo 2,342 raw y 402 válidos
  (`17.16%`); Control 1,528 raw y 225 válidos (`14.73%`). Rechazos por
  brazo (raw): Treatment: duplicado 802, V1 610, V3 92, V4 436; Control:
  duplicado 552, V1 301, V3 75, V4 375. La diferencia 402–225 queda
  explicada por mayor volumen raw de Treatment (814 filas más) y menor
  proporción de solapamiento V1/V4 en Control; no es una medida de relevancia.
  `ARM_YIELD_ASYMMETRY=EXPLAINED` para yield de adquisición.
- **Canonicalización:** `tools/execute_v5_searxng_capture.py` baja scheme y
  host, elimina fragmentos, quita sólo la slash final y preserva íntegramente
  query parameters. En 3,870 filas hubo 0 colisiones de distintas URLs
  originales hacia una misma canónica; transformaciones observadas:
  8 fragmentos, 27 hosts con cambio de case y 1,389 slash finales. No hay
  false collapse demostrado: `CANONICALIZATION_FALSE_COLLAPSE_RISK=LOW`.
- **Hipótesis:**
  `H1_QUERY_UNIVERSE_LOW_EFFECTIVE_DIVERSITY=SUPPORTED` (31 conceptos, 36
  contextos y 8 plantillas repetidas); `H2_SEARXNG_RESULT_CONCENTRATION=SUPPORTED`
  (916 URLs únicas, conjuntos idénticos y concentración documental);
  `H3_HISTORICAL_CORPUS_SATURATION=SUPPORTED` (48.81% de raw rechazado por
  V1/V3/V4); `H4_INTERNAL_DUPLICATION=SUPPORTED` (1,354/3,870=34.99% de
  raw); `H5_CANONICALIZATION_OVER_COLLAPSE=NOT_SUPPORTED` (0 colisiones);
  `H6_SINGLE_PROVIDER_LIMITATION=INSUFFICIENT_EVIDENCE` (el proveedor
  respondió 100% de requests, pero no se puede separar su techo de la
  estrategia); `H7_ARM_SPECIFIC_QUERY_BEHAVIOR=PARTIALLY_SUPPORTED`
  (yield raw asimétrico explicado por volumen/solapamiento, sin inferencia de
  relevancia). Causa primaria: baja diversidad efectiva del universo de
  queries, que amplifica concentración de resultados y saturación histórica.
  Secundarias: duplicación interna y contaminación del corpus histórico.
- **Capas dominantes:** domina `QUERY STRATEGY LIMIT`; `SEARCH ENGINE LIMIT`
  es secundario/no aislable con estos datos; `EVALUATION CORPUS LIMIT` es
  también dominante para el objetivo confirmatorio porque casi la mitad del
  raw fue histórico. El resultado es ambos impactos: experimentalmente V5 no
  tiene potencia confirmatoria; productivamente no prueba que AMATL no halle
  resultados útiles.
- **Decisión:** `ARCHITECTURE_DECISION=PRE_EXECUTION_NOVELTY_DIVERSITY_GATE`.
  Antes de congelar o ejecutar una campaña, AMATL debe estimar novedad contra
  el corpus histórico y diversidad efectiva de queries/result-set esperados,
  y rechazar o sustituir candidatos redundantes hasta satisfacer gates por
  brazo. La compuerta es una sola arquitectura coherente: selección
  estratificada por familia/tema/dominio con presupuesto de novedad; no es una
  iniciativa separada de multi-provider ni una revisión de canonicalización.
  Su efecto esperado es trasladar el rechazo desde después de requests a la
  planificación, reducir duplicación/contaminación y hacer auditable la
  capacidad de llegar a 474/474. Alcance: generador/manifest de queries,
  estimador offline de novelty, estratificación, reporte de gates y bloqueo
  pre-ejecución. No cambiar: código productivo de búsqueda, canonicalizador
  actual, provider routing, ground truths, V1–V5 ni artefactos históricos.
- **Readiness futura:** no considerar otro confirmatorio hasta disponer de
  estimación previa de novelty por brazo, diversidad efectiva y result-set
  overlap, contaminación histórica esperada, yield conservador y balance de
  brazos. Gates mínimos derivados de V5: novelty histórica esperada <20% del
  raw por brazo (V5 fue 48.81% global), duplicación interna esperada <20%
  (V5 fue 34.99%), y capacidad conservadora `>=474` válidos por brazo con
  margen explícito; además, ningún brazo puede proyectar menos del 90% del
  otro sin explicación pre-registrada. El gate debe basarse en estas
  proyecciones y no sólo en raw-result yield histórico.
- **Consecuencias:** Se añade una barrera de planificación y coste de
  instrumentación; pueden descartarse muchas queries nominalmente distintas.
  A cambio, una campaña deja de consumir presupuesto en resultados que el
  propio corpus ya invalida y puede distinguir fallo de adquisición de
  saturación experimental.
- **Siguiente acción única:** implementar la compuerta offline de
  `novelty/diversity` que produzca un manifest bloqueable por brazo antes de
  cualquier request.
- **Trazabilidad:** V5 `final-closure-report.json`, `execution-report.json`,
  `attempt-ledger.json`, `raw-capture.json`, `prelabel-freeze.json`;
  V4 `new-corpus-v1/v4/supplemental-v4-closure-report.json` y
  `supplemental-v4-attempt-ledger.json`; canonicalización en
  `tools/execute_v5_searxng_capture.py`.

### Cierre estructurado post-V5

```text
POST_V5_ANALYSIS_STATUS=COMPLETE_WITH_ARCHITECTURAL_DECISION
STARTING_HEAD=ed88aa1b2f29d465669f0cba956f125aa1a3b252
WORKTREE_INITIAL=CLEAN
FROZEN_QUERIES=2172 (1086 pares)
EXECUTED_QUERIES=2172
RAW_RESULTS=3870
CANONICAL_RESULTS=2516
VALID_RESULTS=627
FINAL_ACCEPTED_RESULTS=627
V5_RAW_RESULTS=3870
V5_VALID_RESULTS=627
V5_VALID_RATE=16.20%
V5_REJECTION_RATE=83.80%
DUPLICATE_CURRENT_RATE=34.99%
HISTORICAL_OVERLAP_RATE=48.81%
UNIQUE_URLS_V5_RAW=916
V5_NEW_UNIQUE_URLS=627
UNIQUE_URLS_V1=555 (reconstrucción por el clasificador histórico congelado)
UNIQUE_URLS_V2=10 (reconstrucción por el clasificador histórico congelado)
UNIQUE_URLS_V3=82 (reconstrucción por el clasificador histórico congelado)
UNIQUE_URLS_V4=382 (reconstrucción por el clasificador histórico congelado)
UNION_HISTORICAL_URLS=923
CORPUS_SATURATION=SEVERE
QUERY_UNIQUENESS=100% exact y normalizada
QUERY_FAMILY_COUNT=31 conceptos base; 36 contextos; 8 formas lingüísticas observadas
EFFECTIVE_QUERY_DIVERSITY=LOW relative to nominal count
RESULT_SET_OVERLAP=27.18% mean paired Jaccard; 844 identical-to-prior sets
DOMAIN_CONCENTRATION=HIGH; top 15 domains=25.89% of raw rows
TREATMENT_VALID_YIELD=17.16%
CONTROL_VALID_YIELD=14.73%
ARM_YIELD_ASYMMETRY=EXPLAINED
CANONICALIZATION_FALSE_COLLAPSE_RISK=LOW
PRIMARY_ROOT_CAUSE=H1_QUERY_UNIVERSE_LOW_EFFECTIVE_DIVERSITY
SECONDARY_ROOT_CAUSES=H2_SEARXNG_RESULT_CONCENTRATION; H3_HISTORICAL_CORPUS_SATURATION; H4_INTERNAL_DUPLICATION
DOMINANT_LIMIT_LAYER=QUERY_STRATEGY_LIMIT (with EVALUATION_CORPUS_LIMIT co-dominant for confirmation)
PRODUCT_IMPACT=B (not a demonstrated product failure)
EXPERIMENTAL_IMPACT=V5 inconclusive; target not reached
FUTURE_CONFIRMATORY_READINESS=NOT_READY_UNTIL_PRE_EXECUTION_GATES_PASS
FILES_CREATED=0
FILES_MODIFIED=decisiones_amatl.md
TESTS_RUN=offline JSON/ledger reconciliation and canonicalization collision audit
TESTS_STATUS=PASS; no network requests
COMMIT_CREATED=YES (hash reported at handoff)
BLOCKERS=none for decision; implementation intentionally deferred
FINAL_DECISION=ACCEPT ADR-012; do not execute V6 or reopen V5
NEXT_SINGLE_ACTION=Implement offline novelty/diversity manifest gate before any request
```

## ADR-013 — Deep acepta targets de Search seleccionados y un tope de fetch por request

- **Fecha:** 2026-09-07
- **Estado:** Aceptada
- **Contexto:** El baseline operacional E2E del 2026-08-31 (`CONTINUIDAD.md`)
  registró dos `KNOWN_NON_BLOCKING_LIMITATION` sobre Deep:
  `DEEP_SELECTED_RESULT_IDS_SUPPORTED=NO` y
  `PER_REQUEST_FETCH_CAP_SUPPORTED=NO`. El único camino de Deep era volver a
  ejecutar Search internamente y adquirir hasta `deep_max_fetches` documentos
  del conjunto resultante, sin que el llamante pudiera acotar ni seleccionar.
  El commit `fdfac71` implementó ambas capacidades y actualizó la superficie
  HTTP y el contrato OpenAPI (`docs/api/openapi.yaml`), pero no dejó entrada
  en este registro; ADR-001 exige una decisión dedicada cuando cambia una API
  pública. Esta ADR cierra ese hueco y no reabre WP-1 ni toca routing,
  ranking, relevancia, providers ni comportamiento Evidence congelados.
- **Decisión:** `AmatlService` gana dos métodos, manteniendo un solo core y la
  invariante «Search no hace fetch; Deep es dueño de toda adquisición de red»:
  - `deep_with_fetch_cap(q, max_fetches, surface)` — camino Search-backed
    existente con un tope opcional por request. El tope **sólo puede
    estrechar** el presupuesto de superficie: `validate_deep_fetch_cap`
    rechaza `0` y cualquier valor `> deep_max_fetches` configurado como
    `InvalidInput`. `deep()` queda como `deep_with_fetch_cap(q, None, surface)`.
  - `deep_selected(q, targets, max_fetches, surface)` — Deep desde identidades
    de `SearchResult` ya mostradas, sin re-ejecutar Search. `DeepTarget` sólo
    transporta `url`, `title` opcional y `provider`; nunca contenido de
    documento. Cada target se valida (`validate_search_url`, provider
    declarado y presente en el registro), se canonicaliza y se deduplica
    contra la URL canónica antes de llegar al `DeepBudget` compartido. El
    número de targets se acota a `[1, deep.top_k]`. La `RoutingRecommendation`
    sintética va vacía con `debug_reasons = ["selected_search_targets"]`: no
    se invoca router, providers ni ranking.
  Superficie: `POST /deep` acepta `DeepInput` (`q`, `targets?`,
  `max_fetches?`) — `GET /deep` sin cambios; MCP `deep` expone `max_fetches`;
  CLI `amatl deep --max-fetches N` usa `deep_with_fetch_cap`. `data_policy`
  sigue gobernando todo egress: el tope reduce fetches, nunca los habilita, y
  la ruta de adquisición (Fetcher/Budget/robots) es la misma. Los secretos
  siguen sólo por variable de entorno.
- **Consecuencias:** El llamante puede pedir una adquisición más barata y
  dirigida sin cambiar la configuración del operador. No cambia el
  comportamiento por defecto: sin `targets` y sin `max_fetches`, `/deep` y
  `amatl deep` se comportan exactamente como en el baseline. `Document` gana
  metadato `search_providers` (procedencia del `SearchResult` que autorizó la
  adquisición); Evidence v2 no lo interpreta y sigue document-grounded. La
  expansión de presupuesto observada en el baseline (`max_fetches=10` desde
  tres resultados) ahora es acotable con `max_fetches` sin tocar el límite
  congelado.
- **Alternativas descartadas:** permitir que el request suba el tope por
  encima del configurado (rompería el control del operador sobre egress);
  aceptar contenido de documento en `DeepTarget` (Deep dejaría de ser el
  único adquiriente y Evidence perdería grounding verificable); re-ejecutar
  Search en `deep_selected` para «confirmar» los targets (gasto de provider
  innecesario y acoplamiento a disponibilidad de fuente).
- **Trazabilidad:** `service.rs::{deep_with_fetch_cap,deep_selected,
  validate_deep_fetch_cap,deep_from_search,DeepTarget}`;
  `deep.rs` (metadato `search_providers`); `amatl-server/src/lib.rs::{deep_post,
  DeepParams}`; `amatl-server/src/mcp.rs`; `amatl-cli/src/main.rs::deep`;
  `docs/api/openapi.yaml` (`DeepInput`, `DeepTarget`);
  `crates/amatl-core/tests/deep_phase5.rs`;
  `crates/amatl-server/src/tests.rs`.
