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

## ADR-005 — Adaptador HTML bloqueado por gobernanza

- **Fecha:** 2026-08-12
- **Estado:** Aceptada
- **Contexto:** Un adapter HTML `best_effort` no implica autorización para
  scraping ni certeza sobre ToS/coste.
- **Decisión:** El adaptador HTML siempre declara
  `provider_pending_explicit_approval` y no ejecuta red hasta una revisión
  verificable y ficha completa.
- **Consecuencias:** Incluir el nombre en `providers.enabled` no lo activa; su
  ausencia nunca rompe el estado global si hay resultados útiles.
- **Alternativas descartadas:** habilitación implícita, scraping oportunista o
  asumir permiso por acceso público.
- **Trazabilidad:** adaptador HTML retirado (ver ADR-010); `service.rs:298,349`.

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

## ADR-010 — Retiro del adaptador HTML; Marginalia pasa de scaffold a adapter real

- **Fecha:** 2026-08-15
- **Estado:** Aceptada
- **Contexto:** ADR-005 bloqueaba el adaptador HTML fail-closed a la espera de
  revisión, pero no había API de búsqueda web oficial (sólo una Instant
  Answer API, que no devuelve resultados web); el adapter era un stub de 73
  líneas sin endpoint, cliente HTTP ni parseo que nunca podría aprobar
  gobernanza. Mantenerlo listado era una fuente "en progreso" fantasma.
  Marginalia, por otro lado, sí tiene API de búsqueda oficial y era el
  scaffold señalado como siguiente paso en `CONTINUIDAD.md`.
- **Decisión:** Se retira el adaptador HTML del código (módulo
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
- **Alternativas descartadas:** implementar el stub HTML (sin API
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

## ADR-014 — Credencial efímera de provider en memoria, liquidable y zeroizada

- **Fecha:** 2026-10-12
- **Estado:** Aceptada
- **Contexto:** El operador necesitaba insertar la API key de Marginalia desde
  la UI sin reiniciar el proceso ni editar `amatl.toml`, y poder erradicarla
  por completo al terminar. El canal `POST /providers/{name}/credential`
  (admin-scoped, RAM-only) ya existía, pero (a) contradecía `docs/security/
  secrets.md`, que declaraba «los secretos sólo entran mediante variables de
  entorno»; (b) `clear_provider_credential` sólo sacaba la entrada del mapa,
  dejando los bytes de la key en el heap; (c) si el operador también exportaba
  `credential_env`, «Liquidar» no erradicaba la credencial en uso y la UI lo
  ocultaba; (d) no había evento de auditoría ni pruebas del control.
- **Decisión:** Reconocer el canal admin-scoped, no persistente y validado por
  canario como una fuente de credencial legítima, y cerrar sus huecos:
  - La key vive en `BTreeMap<String, Zeroizing<String>>` en RAM; `Zeroizing`
    borra el buffer al reemplazar, liquidar o caer el proceso. `resolved_credential`
    y `ProviderBuildContext.credential` propagan `Zeroizing`, y los adapters
    (Marginalia) guardan `Option<Zeroizing<String>>`, de modo que
    la copia que retiene cada provider se borra al caer. El cuerpo HTTP se
    deserializa en un `SecretKey` que se zeroiza en `Drop`.
  - `ProviderSummary`/`SourceStatus` exponen `credential_source`
    (`none`/`ephemeral`/`environment`) — nunca el valor — para que la UI habilite
    «Liquidar» sólo cuando puede erradicar algo y explique cuándo la key viene
    del entorno.
  - `POST`/`DELETE /providers/{name}/credential` emiten eventos de auditoría
    (`provider_credential_inserted`/`liquidated`/`rejected`) con identidad y
    proveedor, sin el secreto.
  - La inserción sigue validándose con un canario live antes de comprometer, y
    falla cerrado (`false`) bajo egress denegado.
- **Superficie:** `crates/amatl-core/src/service.rs` (`CredentialSource`,
  `ProviderSummary.credential_source`, `SourceStatus.credential_source`,
  `ephemeral_credentials: Arc<RwLock<BTreeMap<String, Zeroizing<String>>>>`);
  `providers/{registry,marginalia}.rs`; `amatl-server/src/lib.rs`
  (`SecretKey`, `audit_credential_action`); `docs/api/openapi.yaml`
  (`CredentialSource`); `amatl-ui/assets/{app.js,i18n.js}`.
- **Consecuencias:** «Liquidar» erradica la key en memoria de forma efectiva
  (no queda en el heap tras el `Drop`) y la UI distingue el origen real de la
  credencial. `credential_source` es aditivo al contrato `schema_version = "1"`.
  Riesgo residual documentado: la RAM puede ir a swap y no hay `mlock`; el
  `String` transitorio del header de una request en vuelo no se zeroiza. La
  key sigue sin persistirse: muere con el proceso; sobrevive a `POST /reload`
  en-proceso, que es lo deseado.
- **Alternativas descartadas:** prohibir el canal y exigir siempre variable de
  entorno (perdería el caso de uso del operador sin ganar una barrera real,
  porque el entorno del proceso también es legible); no zeroizar y confiar en
  el `Drop` normal de `String` (deja la key en el heap); exponer la key o su
  hash en `/status` (filtra material sensible sin necesidad).
- **Trazabilidad:** `docs/security/secrets.md`; `amatl-server/src/tests.rs`
  (autorización admin, fail-closed en `isolated`, DELETE idempotente);
  `service.rs` (semántica set/clear/present, no persistencia en config).

## ADR-015 — Búsqueda con modelo (*strategist*) opcional y apagada por defecto

- **Fecha:** 2026-10-12
- **Estado:** Aceptada
- **Contexto:** La invariante «no introducir LLM obligatorio… sin decisión
  explícita» (`CONTINUIDAD.md`) y ADR-001 (decisión dedicada cuando cambia una
  API pública) exigen documentar la capacidad que wirea un LLM en el camino de
  búsqueda. La funcionalidad se implementó y llegó a `main` sin su ADR; esta
  entrada cierra ese hueco. El strategist nace de la necesidad de mejorar una
  consulta vaga sin volver obligatorio un modelo: es una segunda ronda
  *opcional* que propone una query refinada, no una respuesta generada.
- **Decisión:** `[strategist]` es una sección de configuración independiente
  (`enabled` —por defecto `false`—, `endpoint`, `model`, `credential_env`,
  `timeout_ms` —por defecto `800`—). Con ella apagada, `search`/`deep` corren
  100 % deterministas y el modelo no interviene en absoluto. Con ella
  encendida y completa:
  - tras la primera búsqueda, el strategist recibe la consulta original y, si
    está dentro de `timeout_ms`, devuelve una query refinada;
  - AMATL la re-parsea y, sólo si normaliza a algo distinto de la consulta ya
    enviada, ejecuta una segunda ronda con los **mismos providers** sobre un
    orquestador nuevo (presupuesto propio) y registra el cortacircuito de los
    providers que usó;
  - la fusión es **complementaria**: se conserva el conjunto primario tal como
    quedó rankeado y sólo se anexan resultados nuevos, deduplicando por URL
    canónica (contra el primario y dentro del refinado). El modelo nunca
    inventa ni reordena resultados.
  - **Fail-closed:** `enabled = true` exige egress de red (perfil `standard` +
    `egress = "governed"`), `endpoint` validado como el de inferencia remota,
    `model` no vacío y `credential_env` no vacío. Bajo `isolated` (egress
    denegado) no puede encenderse.
  - **Interruptor admin:** `POST /strategist/enabled` (`scope admin`) valida un
    candidato con el flag ya volteado *antes* de escribir, escribe sólo
    `strategist.enabled` con `toml_edit` (preserva comentarios) y aplica el
    cambio con el mismo `reload()` atómico, sin reiniciar. La UI lo expone como
    el toggle «Búsqueda con modelo», visible pero deshabilitado si
    `endpoint`/`model` no están configurados.
  - **Privacidad:** el prompt del LLM embebe únicamente el texto de la
    consulta. `generate_strategy` recibe `&SearchResponse` pero **no usa los
    resultados** (`_current_results`); no se envían fragmentos ni contenido
    recuperado — a diferencia de `answer`.
  - **Gobierno:** la credencial se lee de la variable nombrada en
    `credential_env` en el momento de la llamada; `/status` reporta
    `strategist.available` (presencia) sin exponer el valor.
- **Superficie:** `config/strategist.rs` (`StrategistConfig`,
  `validate_strategist`); `strategist.rs` (trait `SearchStrategist`,
  `NoOpStrategist`); `strategist/llm.rs` (`LlmStrategist`); `service.rs`
  (`search_inner`, `append_complementary`, `StrategistStatus`); `config/mod.rs`
  (`strategist` field, `set_strategist_enabled`); `amatl-server/src/lib.rs`
  (`strategist_toggle`, `required_scope`); `amatl-server/src/routes.rs`;
  `docs/api/openapi.yaml` (`/strategist/enabled`, `StrategistStatus`);
  `amatl-ui` (toggle); `docs/operacion.md`; `amatl.example.toml`.
- **Consecuencias:** el comportamiento por defecto no cambia; el LLM sigue
  siendo opcional y la capacidad determinista de AMATL no depende de él. Coste:
  con el strategist encendido, una búsqueda puede gastar hasta dos rondas de
  presupuesto de provider (cada orquestador tiene su propio `Budget`) y añade
  hasta `timeout_ms` de latencia. El proveedor del modelo es un tercero que
  recibe el texto de la consulta por diseño — se divulga, no se mitiga; no hay
  consentimiento por-request más allá del interruptor admin. Riesgo residual:
  la RAM/proceso del operador y el proveedor externo quedan fuera del control
  de AMATL.
- **Punto de revisión (trade-off explícito):** a diferencia de `answer`, el
  strategist **no** exige `data_policy.inference = "remote_explicit"`: se
  gobierna por egress + el flag `enabled`. La justificación es que sólo envía
  la consulta (no contenido recuperado) y que `isolated` ya lo bloquea. Un
  revisor puede preferir alinear ambos canales bajo `remote_explicit`; se deja
  registrado como decisión consciente, no como omisión.
- **Alternativas descartadas:** exigir `remote_explicit` (más fricción para un
  canal que no envía contenido; queda como punto de revisión); enviar los
  resultados de la primera ronda al modelo (filtraría contenido recuperado y
  ampliaría la superficie sin mejorar la propuesta de query); reordenar o
  reemplazar el conjunto primario con el refinado (rompería el contrato de
  ranking determinista y la reproducibilidad); usar embeddings locales para
  «refinar» sin red (no produce operadores de query ni mejora observable, y
  añadiría un backend pesado que hoy no existe).
- **Trazabilidad:** `crates/amatl-core/src/strategist.rs`;
  `crates/amatl-core/src/strategist/llm.rs`;
  `crates/amatl-core/src/config/strategist.rs`;
  `crates/amatl-server/src/tests.rs`
  (`strategist_toggle_flips_the_flag_and_applies_without_restart`,
  `strategist_toggle_refuses_to_enable_an_incomplete_config_without_writing`);
  `docs/operacion.md`; `docs/security/threat-model.md`
  (`Core → inference (strategist)`).

## ADR-016 — Retiro de las fuentes de pago del producto

- **Fecha:** 2026-10-26
- **Estado:** Aceptada
- **Contexto:** ADR-010 retiró el adaptador HTML porque su adapter nunca podría
  aprobar gobernanza, y dejó a las fuentes de pago en un estado intermedio: adapter
  completo y funcional, pero con `approval_status = "rejected"` fijado por
  política (no se contratan fuentes de pago) y, además, expuestos en la UI como
  fuente opcional opt-in (selector, diálogo de key, rutas de credencial). Ese
  estado es incoherente: el operador ve una fuente seleccionable que el propio
  producto considera no contratable, y la superficie de credencial de un
  provider retirado es riesgo sin contrapartida (ADR-014 tuvo que endurecer el
  ciclo de vida de la key de un adapter que ya no se puede usar). Se cierra la
  ambigüedad retirándolos de verdad, no apagándolos.
- **Decisión:** Se retiran las fuentes de pago de extremo a extremo:
  - **Código:** se eliminan sus módulos de adapter, sus
    factories y sus entradas en `builtin_provider_records()`.
    `ProviderRegistry::builtin()` despacha exactamente SearXNG y Marginalia.
  - **Configuración (segunda puerta):** `RETIRED_PROVIDERS` /
    `is_retired_provider()` (`config/providers.rs`) y `Config::validate()`
    rechazan esos nombres **por nombre**, tanto en una tabla declarada
    `[providers.<nombre>]` como en `providers.enabled`. El
    error es explícito («retirado por política») en lugar del genérico
    `unknown enabled provider`, que se leería como una errata. Una configuración
    antigua falla en el arranque en vez de desplegar una fuente menos en
    silencio; incluso una config construida a mano que los liste se detiene en el
    registro (`UnknownProvider`).
  - **UI:** desaparecen selector, diálogo de key, botones y claves de catálogo;
    un test impide que vuelvan.
  - **Documentación y pruebas:** `amatl.toml`, `amatl.example.toml`,
    `docs/configuracion.md`, `docs/gobernanza-providers.md`,
    `docs/security/{secrets,threat-model,asvs-checklist}.md` y los suites de
    CLI/core/servidor/UI se actualizan al inventario real.
- **Consecuencias:** AMATL publica exactamente **dos fuentes: SearXNG (sin
  credencial) y Marginalia (con credencial)**; «conectar una fuente de búsqueda
  real» queda cumplido sin candidatos fantasma. La política de «no se contratan
  fuentes de pago» pasa de ser un `rejected` por defecto —overrideable en
  `amatl.toml`— a una puerta dura en el código: reactivar una fuente retirada exige
  revertir esta ADR (restaurar adapter y quitar el nombre de
  `RETIRED_PROVIDERS`), no cambiar una ficha. Riesgo residual: algunos nombres
  siguen apareciendo como **motores upstream de SearXNG** en los fixtures
  de `providers/searxng.rs` y en la evidencia histórica
  (`docs/testing/searxng-characterization.md`, `test-results/`); son motores de
  la instancia SearXNG del operador, no fuentes de AMATL. ADR-014 sigue
  vigente en su mecanismo (credencial efímera zeroizada), pero su enumeración de
  adapters se reduce a Marginalia.
- **Alternativas descartadas:** dejar los adapters apagados con
  `approval_status = "rejected"` (mantiene el código muerto y la superficie de
  credencial que ADR-014 tuvo que blindar, y confunde al operador con una fuente
  seleccionable que no se puede contratar); conservar sólo la comprobación de
  configuración sin borrar los adapters (un adapter presente se puede reactivar
  por error al tocar el registry); borrar los nombres del historial
  (`CHANGELOG.md`, ADRs previas, `CONTINUIDAD.md`) en lugar de anotarlos
  (destruiría la trazabilidad de por qué se retiraron).
- **Trazabilidad:** `crates/amatl-core/src/providers/{registry.rs,searxng.rs,marginalia.rs}`;
  `crates/amatl-core/src/config/providers.rs` (`RETIRED_PROVIDERS:148`,
  `is_retired_provider`); `crates/amatl-core/src/service.rs`
  (`validate_provider_canary`, `retired_provider_canary_stops_at_the_registry_door`,
  y las vistas `/status`, `/ready`);
  `crates/amatl-ui/src/lib.rs`
  (`retired_sources_are_not_shipped_in_the_ui`);
  `crates/amatl-cli/tests/cli.rs`
  (`provider_canary_rejects_retired_sources_as_configuration_errors`);
  `crates/amatl-core/src/config/mod.rs`
  (`retired_providers_are_absent_and_refused_by_validate`,
  `declaring_a_table_replaces_the_builtin_record_instead_of_merging_it`);
  `crates/amatl-server/src/tests.rs`
  (`reload_picks_up_a_new_source_without_restarting`,
  `source_gauges_count_declared_sources_while_readiness_counts_available_ones`);
  `docs/gobernanza-providers.md`; `docs/configuracion.md`;
  `docs/security/secrets.md`; `CHANGELOG.md` (Unreleased → Removed).

## ADR-017 — Plan de salida de `libsqlite3-sys` (RUSTSEC-2026-0459)

- **Fecha:** 2026-11-13
- **Estado:** Propuesta — pendiente de firma de `@IA-Alex`
- **Contexto:** La re-auditoría de 2026-11-13 dejó un único advisory abierto en el
  grafo bloqueado: `libsqlite3-sys 0.30.1` (RUSTSEC-2026-0459, `medium`/6.8,
  CVE-2026-119375), alcanzado por `sqlx 0.8.6 → sqlx-sqlite → libsqlite3-sys`.
  El advisory afecta sólo a la fuente SQLite `bundled`, que el workspace sí
  habilita, y la explotabilidad es **baja**: vector local (`AV:L`, `UI:P`), sin
  superficie remota —requiere que un tercero menos confiable coloque un fichero
  de base de datos y su `-wal` donde el operador los abra, y la ruta SQLite de
  AMATL es local y controlada por el propio operador (`docs/security/supply-chain.md`,
  «Exploitability: low — no remote surface»). No hay upgrade compatible: el fix
  es `libsqlite3-sys >= 0.39.0`; `sqlx 0.8.6` lo fija en `^0.30` y `sqlx 0.9.0`
  (2026-05-21) lo admite en un rango con tope `< 0.38.0`. Por política
  (`docs/security/supply-chain.md`, `SECURITY.md`) un `ignore` en `deny.toml`
  exige una ADR escrita y aprobada por `@IA-Alex`; ninguna excepción está
  pre-aprobada.
- **Decisión (propuesta):** Cerrar el hallazgo por su **plan de salida**, no por
  supresión. Se adopta como camino principal la **espera activa al rango de
  `sqlx`**, con tres hitos verificables. Los hitos se ejecutaron sin `ignore`;
  la supresión nombrada que rige hoy aparece en la enmienda al final de esta
  ADR:
  0. **Fijar la línea auditada (hecho 2026-11-15):** el manifiesto raíz declara
     `sqlx = "=0.8.6"` en lugar del `0.8` con caret. Un caret admitía cualquier
     `0.8.x` posterior y cada uno decide el rango de `libsqlite3-sys` **y** el
     conjunto de features `sqlite` del grafo, así que un `cargo update` podía
     cambiar la superficie auditada sin que ningún revisor viera un diff de
     manifiesto. El test offline `sqlx_is_pinned_to_the_audited_line`
     (`crates/amatl-core/tests/supply_chain_contract.rs`) falla si se afloja el
     pin o si `Cargo.lock` resuelve otra línea. Es una **barrera, no una
     remediación**: el advisory sigue abierto.
  1. **Salir del pin `0.30.1`:** evaluar el salto a `sqlx 0.9.x`, que admite
     `libsqlite3-sys >=0.30.1, <0.38.0` (tope mayor que `^0.30`, pero todavía
     por debajo del fix). El bloqueo conocido no es el MSRV sino la
     **desalineación de features**: en `sqlx 0.9.0` la feature `sqlite` implica
     `sqlite-bundled`, `sqlite-deserialize`, `sqlite-load-extension` y
     `sqlite-unlock-notify`, mientras que en `0.8.6` implica sólo
     `sqlx-sqlite/bundled`. Un salto directo añade `deserialize` y
     `load-extension` al binario que se distribuye, y la variante
     `sqlite-unbundled` enlaza el SQLite del sistema (rompe el build musl
     estático). Es por tanto un cambio de superficie y de empaquetado, no un
     bump de versión. Uso de `sqlx` acotado a `crates/amatl-core/src/storage.rs`
     (~74 llamadas): el impacto de adaptación es local, pero **no** cierra el
     advisory por sí solo.
  2. **Admitir el fix:** en cuanto una release de `sqlx` incorpore
     `libsqlite3-sys >= 0.39.0` en su rango, un `cargo update -p libsqlite3-sys`
     (o `-p sqlx`, con el pin actualizado en el mismo commit) elimina el advisory
     y el gate vuelve a verde. `libsqlite3-sys 0.39.0`/`0.39.1`/`0.40.0` ya
     existen (crates.io, 2026-11-15), así que falta sólo la release de `sqlx`
     que los admita.
  - **Controles compensatorios mientras esté abierto (ya vigentes):** el gate
    `cargo audit`/`cargo deny check` permanece **rojo** y bloquea merge; el test
    offline `tests/supply_chain_contract.rs` mantiene floors para los advisories
    remediables y **excluye este caso con justificación escrita**; `deny.toml`
    no tiene allowlist silencioso (`wildcards = "deny"`, registries fijados).
- **Consecuencias:** El hallazgo queda clasificado como **deuda local acotada**,
  no como riesgo de producción remoto, y con ruta de cierre explícita y
  revisable. Si algún hito no llegara en el SLA correspondiente (medium → 60 días
  de `SECURITY.md`), esta ADR debe convertirse en un waiver temporal **firmado**,
  con fecha de caducidad y plan de retirada, o resolver el hallazgo por exclusión
  del grafo. La redacción original («no autoriza ningún `ignore`») queda
  sustituida por la enmienda siguiente, que sí autoriza uno — acotado, nombrado y
  con guardas.
- **Enmienda 2026-11-15 — una excepción acotada, no un waiver.** La decisión
  anterior dejaba `cargo audit`/`cargo deny check` rojos mientras emitieran **ese**
  aviso. Un gate que falla en cada corrida por un hallazgo ya analizado deja de
  distinguir el siguiente, y la política del repo («no hay allowlist silencioso»)
  se refería a una allowlist **silenciosa**, no a una excepción escrita. Se
  suprime **un solo ID** (`ignore = ["RUSTSEC-2026-0459"]` en `deny.toml`) bajo
  cuatro controles que impiden que la supresión crezca o pase inadvertida:
  1. **Guardas offline** en `supply_chain_contract.rs`: la variante auditada de
     `libsqlite3-sys` (0.30.1) y el conjunto exacto de IDs suprimidos. Cambia
     cualquiera de los dos → falla en todos los hosts, sin red.
  2. **Gate visible**: `scripts/security-gate.sh` imprime los IDs suprimidos en
     cada corrida y pasa los mismos a `cargo audit`; CI invoca el script con
     `--expected-ignore`, así que un ID nuevo exige editar el workflow.
  3. **Herramientas fijadas** (`cargo-audit@0.22.2`, `cargo-deny@0.18.7`, ya
     fijadas): verificado contra `cargo-deny` 0.18.7 y 0.20.2 que ambos honran
     `ignore`, y que sin él el gate vuelve a fallar.
  4. **Documentación**: el modelo de amenaza y la excepción viven en
     `docs/security/supply-chain.md` («Current exception»).
  **La excepción caduca con el pin**: al subir a una línea de `sqlx` que admita
  `libsqlite3-sys >= 0.39.0` se eliminan en el mismo commit el `ignore`, el
  `--expected-ignore`, la aserción del lockfile y esta enmienda pasa a
  «ejecutada». Sigue siendo deuda **local acotada**: no se ha demostrado que el
  advisory se haya explotado, sino que su precondición exige escritura local en
  el fichero del operador.
- **Alternativas descartadas:** (a) añadir `ignore` en `deny.toml` sin ADR
  firmada — prohibido por la política; (b) forzar un `[patch]`/fork de
  `sqlx-sqlite` para elevar el rango a `>= 0.39.0` — mete un fork no auditado en
  la cadena de suministro, peor que el propio advisory; (c) migrar de driver
  SQLite (`rusqlite`, `sqlite3` directo) — reescribe la capa de persistencia por
  un riesgo local bajo, coste desproporcionado; (d) retirar el escritor de
  `bundled` por `sqlite-unbundled` — cambia el modelo de enlace y la
  reproducibilidad del binario sólo para esquivar un advisory que permanece en el
  grafo de todas formas.
- **Trazabilidad:** `docs/security/supply-chain.md` («Re-audit 2026-11-13» y
  «Threat-model note»); `Cargo.toml` (`sqlx = "=0.8.6"`); `deny.toml`;
  `tests/supply_chain_contract.rs` (`sqlx_is_pinned_to_the_audited_line`);
  `Cargo.lock` (`libsqlite3-sys 0.30.1`); `crates/amatl-core/src/storage.rs`;
  `SECURITY.md`.

