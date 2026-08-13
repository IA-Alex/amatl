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
