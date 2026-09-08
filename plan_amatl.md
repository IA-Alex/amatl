# AMATL — Especificación canónica (golden_template)

Única fuente normativa del proyecto. Todo desarrollo, revisión, generación de código, testing y modificación se valida contra este documento. `decisiones_amatl.md` es ADR/changelog histórico y no es fuente activa.

Semántica normativa:

- `MUST` — requisito obligatorio.
- `MUST NOT` — prohibición.
- `SHOULD` — recomendado salvo justificación explícita.
- `MAY` — opcional.

---

## 1. Definición del producto

AMATL es un buscador generalista multi-fuente, Linux-first, rápido, extensible, modular y resistente a fallos.

- Nombre del producto: `AMATL`.
- Versión: `AMATL v1` / `amatl 1.0.0`.
- Identificador técnico: `amatl-search`.

Flujo visible del producto: `buscar → revisar → abrir`.

AMATL **MUST NOT** ser:

- chatbot;
- generador de texto;
- crawler masivo;
- dashboard analítico;
- agente autónomo;
- sistema dependiente de LLM;
- plataforma dependiente de un único proveedor.

---

## 2. Invariantes y objetivos

### 2.1 Invariantes del producto (MUST)

1. Buscador generalista.
2. Search rápido y ligero.
3. Multi-provider.
4. Linux-first.
5. Sin dependencia obligatoria de LLM.
6. Deep opcional.
7. Trafilatura y Chromium fuera del camino crítico.
8. Degradación controlada.
9. Un único core para CLI, UI, API y MCP.
10. Budget global con ownership exclusivo del orquestador.
11. Contratos explícitos.

### 2.2 Objetivo

Permitir búsquedas generales sobre múltiples fuentes sin obligar al usuario a seleccionar manualmente motores o fuentes.

### 2.3 Prioridades

- cobertura útil;
- baja latencia;
- resultados únicos;
- diversidad de dominios;
- independencia de providers;
- degradación controlada;
- bajo acoplamiento;
- seguridad desde diseño;
- simplicidad operativa;
- interfaz mínima;
- funcionamiento nativo en Linux.

### 2.4 Restricciones iniciales (MUST NOT introducir)

Elasticsearch, OpenSearch, Neo4j, vector DB externa, framework web pesado, crawler masivo, agent loops, LLM obligatorio, ranking opaco, provider crítico único, dependencia irreversible de Trafilatura, dashboards complejos, lógica duplicada entre superficies.

---

## 3. Stack tecnológico

- Lenguaje principal: **Rust**.
- Runtime y concurrencia: **Tokio**.
- HTTP: **Reqwest**.
- Serialización: **Serde** + **JSON**.
- CLI: **Clap**.
- Persistencia: **SQLite** + **SQLx**.
- Observabilidad: **tracing** + **tracing-subscriber**.
- Errores: **thiserror** + **anyhow**.
- URLs: **url**.
- Parsing HTML / SERP: **scraper** (parsing de SERP, HTML estructural simple, adapters específicos; no extractor editorial principal).
- Extracción de contenido: **Trafilatura** — capability opcional, exclusivamente en Deep, inicialmente mediante CLI, salida Markdown o JSON con metadata. Debe permanecer detrás de la interfaz reemplazable `Extractor` (Trafilatura | Native | Alternative).
- Hashing: **sha2**.
- Configuración: **TOML** + variables de entorno para secretos.
- API: **Axum** (superficie futura).
- MCP: superficie futura sobre el mismo core.
- Build/QA normativo: **Cargo**, **rustfmt**, **Clippy**, **Cargo Audit**, **Cargo Deny**, **SBOM**.

Tooling de autor (VS Code, Claude Opus, DeepSeek Pro) no forma parte de esta especificación; se documenta en `DEVELOPMENT.md`.

---

## 4. Sistema operativo y distribución

Objetivo inicial: **Linux**.

Compatibilidad prioritaria: 1) Debian, 2) Ubuntu, 3) Arch Linux. Posteriormente: Fedora y otras distribuciones.

CLI y core **MUST NOT** tener dependencias gráficas obligatorias.

Distribución:

- **Releases precompilados musl** como vía principal.
- `cargo install` como vía alternativa desde source.
- Backend TLS: **rustls** (compatible con binario estático musl). Evitar OpenSSL en la distribución estática principal; cualquier desviación **MUST** documentarse.
- Paquetes `.deb` / `.rpm` / AUR como integración nativa por distro (Debian, Ubuntu, Arch prioritarios).

---

## 5. Arquitectura modular

```
amatl/
 ├── core/
 ├── query/
 ├── classify/
 ├── planning/
 ├── providers/
 ├── router/
 ├── execution/
 ├── normalize/
 ├── canonical/
 ├── dedupe/
 ├── ranking/
 ├── diversity/
 ├── budget/
 ├── deep/
 ├── fetch/
 ├── render/
 ├── extract/
 ├── gaps/      (implementado)
 ├── cache/
 ├── storage/
 ├── telemetry/
 ├── security/
 ├── cli/
 ├── api/
 ├── mcp/
 └── ui/
```

La lógica funcional **MUST** permanecer fuera de CLI, UI, API y MCP. Todas las superficies **MUST** consumir el mismo core. `gaps/` y la entidad `Evidence` se planificaron como stubs post-MVP; ambos están implementados desde la fase 5 (`gaps.rs`, `evidence.rs`) y este documento se mantiene como registro de la normativa original.

### 5.1 Ownership de la orquestación

- `router/` sólo recomienda estrategia (criterios de selección, prioridades, solicitudes de capacidad); **MUST NOT** asignar Budget definitivo.
- `planning/` representa y construye `SearchPlan` (snapshot de la estrategia acordada).
- `execution/` **MUST** contener `SearchOrchestrator` y `DeepOrchestrator`: posee el Budget, aplica el `deadline`, coordina providers y decide la ejecución de SubQuery. El ownership del Budget es exclusivo del orquestador (invariante 10).
- `fetch/`, `render/` y `extract/` permanecen como módulos top-level, pero son capacidades exclusivas orquestadas por Deep; Search **MUST NOT** invocarlas.

---

## 6. Ciclo de vida de entidades

Flujo canónico:

```
Query → Classification → SearchPlan → ProviderResult → NormalizedResult
→ CanonicalResult → DeduplicatedResult → SearchResult
→ Document → Evidence → Gap → SubQuery
```

- `Query`: intención humana.
- `Classification`: intención derivada por heurística léxica.
- `SearchPlan`: estrategia de ejecución.
- `ProviderResult`: salida directa del adapter.
- `NormalizedResult`: modelo común.
- `CanonicalResult`: URL original y canónica.
- `DeduplicatedResult`: recurso consolidado.
- `SearchResult`: salida final de Search.
- `Document`: contenido enriquecido en Deep.
- `Evidence`: señal derivada del Document (implementado).
- `Gap`: déficit observable (implementado).
- `SubQuery`: propuesta de expansión.

`SearchResult` **MUST NOT** almacenar el cuerpo completo; el cuerpo pertenece a `Document`.

Estructuras persistidas o expuestas externamente **MUST** incluir `schema_version`.

Entidades de valor: `OriginalUrl`, `CanonicalUrl`, `FinalUrl`, `Rank`, `RankingScore`, `SearchStatus`, `ProviderError`, `ProviderCapabilities`. `RankingScore`, `SearchStatus`, `ProviderError` y `ProviderCapabilities` son los nombres canónicos de score de ranking, estado global de búsqueda, error de provider y capacidades de provider, respectivamente.

Los nombres **MUST** mantenerse consistentes en Rust, JSON, SQLite, CLI, logs y documentación. Correspondencia normalizada nombre físico (módulo) ↔ conceptual: `normalize/` → Normalization, `canonical/` → Canonicalization, `dedupe/` → Deduplication. **MUST NOT** alternar arbitrariamente con Normalizer/Canonicalizer/Deduplicator.

---

## 7. Contratos por módulo

### 7.1 Query

Responsabilidad: representar exactamente lo solicitado por el usuario. El parser es el único módulo autorizado para interpretar texto libre y operadores.

Campos: `raw_query`, `normalized_query`, `quoted_terms`, `excluded_terms`, `domains`, `excluded_domains`, `file_types`, `language`, `region`, `date_from`, `date_to`, `warnings`.

Operadores: `site:`, `-site:`, `filetype:`, `lang:`, `region:`, `before:`, `after:`, `exact:`.

Reglas:

- `raw_query` **MUST NOT** modificarse.
- Providers **MUST NOT** reinterpretar texto libre.
- Filtros inválidos **MUST** generar warning o tratamiento literal.
- Contradicciones se resuelven mediante política explícita.
- Los fallos detectables en fronteras contractuales **MUST** producir `warning`, `degradation` o `error` tipado.

### 7.2 Classification

Categorías: `general`, `technical`, `code`, `documentation`, `news`, `academic`, `commercial`, `forum`, `social`, `media`, `navigation`.

Salida: `primary_category`, `secondary_categories`, `confidence`, `confidence_by_category`, `reasons`.

- `confidence`: confianza de la categoría **primaria** (escalar 0–1).
- `confidence_by_category`: mapa categoría → confianza (0–1).
- MVP determinista: valores derivados de heurística léxica, sin recalibración por modelo.

Política:

- una categoría primaria obligatoria;
- máximo dos secundarias;
- confianza por categoría;
- fallback a `general`;
- comportamiento determinista en MVP.

Prioridad: `operadores explícitos > filtros explícitos > heurísticas léxicas`.

El clasificador **MUST NOT** consultar providers, usar telemetría ni ejecutar routing.

Las categorías de Classification y el enum `result_type` (§7.9) son **taxonomías distintas**: Classification describe la intención de la Query; `result_type` describe la naturaleza del resultado. **MUST NOT** asumirse correspondencia 1:1 obligatoria entre ambas.

### 7.3 SearchPlan

Responsabilidad: representar cómo AMATL decidió ejecutar la búsqueda.

Contenido: `query`, `classification`, `selected_providers`, `provider_priority`, `provider_budgets`, `global_budget`, `fallback_policy`, `expansion_policy`, `stop_conditions`, `debug_reasons`.

`provider_budgets` es el snapshot final de las reservas creadas por el orquestador; el ownership del Budget **MUST** permanecer en el orquestador. El router sólo produce `provider_budget_requests` (necesidades estimadas), nunca presupuesto definitivo.

Reglas:

- Modificar routing **MUST NOT** modificar Query.
- Providers **MUST NOT** modificar SearchPlan.
- Ejecución consume SearchPlan.
- Decisiones **MUST** ser reproducibles en debug.

### 7.4 Provider

Cada fuente implementa un adapter. Responsabilidad: traducir Query estructurada a una fuente externa y devolver resultados.

**MUST NOT** decidir: ranking global, deduplicación global, diversidad, expansión, routing.

Capabilities (`ProviderCapabilities`): `pagination`, `language`, `region`, `time_range`, `site_filter`, `file_filter`, `news`, `code`, `docs`, `academic`, `authentication`, `estimated_cost`.

Entrada: Query, filtros, `deadline`, Budget asignado, límites.

Salida: resultados, filtros aceptados, filtros ignorados, filtros aproximados, estado.

Errores: `timeout`, `rate_limit`, `auth`, `network`, `invalid_response`, `parser_error`, `quota`, `unavailable`.

Reglas:

- Resultados parciales son válidos.
- **MUST NOT** incluir secretos ni headers sensibles en errores.
- **MUST NOT** alterar resultados por lógica global.

`provider_rank` (opcional): posición que el provider reporta dentro de sus propios resultados. **MAY** derivarse sólo si el provider reporta o preserva un orden nativo verificable; en caso contrario **MUST** ser `null`/ausente. AMATL **MUST NOT** inventarlo. `deadline` lo define el orquestador y se propaga como límite duro; ningún provider fija su propio deadline.

### 7.5 Provider Router

Entrada: Query, Classification, capabilities, provider health, telemetría, Budget.

Salida: SearchPlan.

Criterios: categoría, idioma, región, filtros, disponibilidad, latencia, error rate, unique result ratio, duplicate ratio, top-K contribution, coste, ganancia marginal.

Reglas:

- El router produce `provider_budget_requests` (necesidades estimadas); **MUST NOT** asignar Budget definitivo (ownership del orquestador).
- Fallback estático cuando no hay telemetría.
- Telemetría ajusta prioridades, no elimina reglas base.
- Ningún provider domina sólo por volumen.
- Decisiones explicables en debug.

### 7.6 Provider Value

Estados:

- `Bootstrap`: telemetría insuficiente; routing estático.
- `Learning`: telemetría parcial; ajustes suaves.
- `Mature`: telemetría suficiente; priorización adaptativa dentro de límites.

Variables: `unique results`, `duplicate ratio`, `top-K contribution`, `success rate`, `timeout rate`, `latency`, `diversity`, `cost`.

Reglas:

- Métricas globales y por categoría.
- Ventana temporal y decaimiento de datos antiguos.
- Muestra mínima antes de cambiar de estado.
- Timeout penaliza más que latencia.
- Providers nuevos **MUST** mantener cuota mínima de exploración.
- Telemetría **MUST NOT** modificar límites de seguridad.

### 7.7 Budget

El Budget **MUST** pertenecer exclusivamente al orquestador. Los módulos sólo solicitan capacidad, consumen reservas y reportan consumo; **MUST NOT** crear o ampliar límites.

Presupuesto global: tiempo máximo, coste máximo, providers máximos, subqueries máximas.

Presupuesto por etapa: Search, Fetch, Render, Extract, Crawl, Gap expansion.

Presupuesto por recurso: provider, dominio, bytes, redirects, browser calls, crawl URLs.

Causas estandarizadas: `time_exhausted`, `deadline_near`, `provider_limit`, `fetch_limit`, `byte_limit`, `redirect_limit`, `browser_limit`, `crawl_limit`, `cost_limit`, `subquery_limit`.

Agotar presupuesto **MUST** producir resultado parcial siempre que existan resultados útiles.

### 7.8 Parallel Search

Responsabilidad: ejecutar SearchPlan.

Controla: timeout individual, deadline global, concurrencia global, concurrencia por provider, retries, cancelación, resultados parciales.

No interpreta contenido ni ranking.

Retries sólo para errores recuperables, con **`Retry-After`**, **backoff exponencial** y **jitter**; máximo 1–2 reintentos.

**MUST NOT** reintentar automáticamente: auth inválida, parser determinista, configuración inválida, bloqueo explícito.

Salida: `provider_results`, `providers_used`, `providers_failed`, `providers_partial`, `elapsed`, `budget_remaining`.

Un provider lento **MUST NOT** bloquear el resto.

`deadline` lo define el orquestador; Parallel Search lo hace cumplir mediante cancelación. `timeout individual` es el límite por defecto de cada provider.

### 7.9 Normalization

Convierte ProviderResult al modelo común.

Campos mínimos: `title`, `url`, `provider`, `result_type`.

`url` es la URL reportada por el provider (pre-canonicalización); Canonicalization la consume para derivar `original_url` y `canonical_url`, que son los únicos campos de URL expuestos en SearchResult.

Opcionales: `provider_rank`, `snippet`, `published_at`, `author`, `language`, `file_type`, `thumbnail`, `metadata`.

`result_type` usa el enum: `organic`, `news`, `media`, `document`, `code`, `forum`, `social`, `commercial`, `navigation`, `other`. Default `organic` cuando el provider no reporta tipo; `other` para tipos no clasificables.

**MUST** distinguir valor reportado por provider de valor derivado por AMATL.

Reglas: limpiar entidades HTML, normalizar encoding, limpiar whitespace, URL inválida no llega al fetcher, no inventar metadata.

### 7.10 Matriz de normalización degradada

| Condición | Acción |
|---|---|
| URL inválida | Descartar resultado |
| Esquema no permitido | Descartar resultado |
| Título ausente | Conservar; usar dominio/path como fallback visual |
| Snippet ausente | Conservar |
| Snippet corrupto | Eliminar sólo snippet |
| Fecha inválida | `published_at = null`; conservar original en metadata |
| Encoding defectuoso | Reparar; si falla, eliminar sólo campo afectado |
| Metadata parcial | Conservar campos válidos |
| Respuesta inesperada | `invalid_response` |
| Resultado parcial del provider | Conservar |
| Canonicalización incompleta | Conservar original y marcar degradación |

Semántica: `error`, `warning`, `degradation` **MUST NOT** tratarse como equivalentes.

### 7.11 Canonicalization

Salida: `original_url`, `canonical_url`, `transformations`, `status`.

**MUST** ejecutarse antes de hash, caché, dedupe y ranking.

Eliminar de forma segura: `utm_*`, `fbclid`, `gclid`, `msclkid`, `yclid`, `_ga`, `_gl`, `mc_cid`, `mc_eid`.

**MUST NOT** eliminar globalmente: `ref`, `source`, `campaign`, `medium`, parámetros ambiguos.

Reglas:

- host en minúsculas;
- `scheme` en minúsculas;
- IDN → punycode;
- eliminación de fragmentos (`#`) cuando no sean semánticos;
- puertos estándar explícitos;
- percent-encoding conservador (no normalizar agresivamente);
- **MUST NOT** resolver redirects;
- **MUST NOT** asumir HTTP = HTTPS;
- slash final sólo mediante regla segura;
- conservar siempre `original_url`.

`canonical_url` pertenece a **Search** (URL normalizada, sin resolución de redirects).

`domain` es un campo derivado exclusivamente de `canonical_url`; **MUST NOT** introducir una fuente independiente de verdad.

### 7.12 Deduplication

Orden: 1) URL exacta, 2) canonical URL, 3) similitud de título, 4) similitud de contenido sólo en Deep.

Estados: `confirmed_duplicate`, `possible_duplicate`, `distinct`.

El resultado consolidado conserva: providers, provider ranks, URLs originales, título seleccionado, snippets alternativos cuando aporten valor, fechas observadas, motivo de fusión.

Un título similar **MUST NOT** bastar por sí mismo para fusionar. La dedupe en Search usa `original_url` + `canonical_url`; `final_url` **MUST NOT** participar en dedupe ni en caché de Search.

### 7.13 Ranking MVP

Política versionada: `ranking_policy = v1`.

Señales del score combinado: RRF, coincidencia query/título, coincidencia query/snippet, frescura, acuerdo entre providers.

RRF **MUST** usarse sólo como señal del score combinado y **MUST NOT** reutilizarse como tie-break.

Requisitos: determinista, explicable, sin LLM, sin embeddings, sin authority score opaco.

Empates: `combined_score → title_match → stable_order`.

**MUST NOT** usar `snippet_match`, RRF ni Diversity como desempate (evita doble ponderación y reglas nuevas innecesarias).

Debug puede mostrar: aportación RRF, señales textuales, frescura, provider agreement, score final, tie-break.

### 7.14 Diversity

Diversity es **etapa post-ranking**, no criterio del ranking y no participa en los empates de score.

Límites suaves por (propiedades del resultado): **dominio**, **provider**, **result_type**.

Estados: `visible`, `relegated_by_diversity`.

Reglas:

- no elimina resultados;
- puede relegarlos;
- alta relevancia puede superar límite mediante umbral versionado;
- resultados relegados siguen disponibles.

Métricas: unique domains, unique providers, unique result types.

### 7.15 Búsqueda progresiva

Primera ronda: 2–3 providers.

Expandir cuando: resultados únicos insuficientes, diversidad baja, cobertura pobre, ganancia marginal esperada suficiente.

Detener cuando: cobertura suficiente, ganancia marginal baja, Budget agotado, deadline cercano, providers adicionales devuelven principalmente duplicados.

### 7.16 Ganancia marginal

Métrica básica: `new_unique_results / provider_query`.

Providers posteriores **MAY** omitirse cuando su valor esperado sea mínimo.

### 7.17 Deep

Deep enriquece SearchResult; no redefine Search.

Entrada: Query, SearchPlan, top-K SearchResult, Budget restante.

Salida: Documents, errores parciales, Ranking v2, Gaps, posibles SubQueries.

Reglas:

- fallo de Deep **MUST NOT** invalidar Search;
- fallo de extracción conserva SearchResult;
- sólo URLs aprobadas por seguridad pueden descargarse;
- browser es excepcional;
- Deep consume Budget restante.
- `fetch`/`render`/`extract` son capacidades exclusivas orquestadas por Deep; Search **MUST NOT** invocarlas.

### 7.18 Fetcher

Responsabilidad: descargar contenido HTTP de forma segura.

Entrada: URL aprobada, timeout, byte limit, redirect limit, headers permitidos.

Salida: `final_url`, `status`, `headers_safe`, `content_type`, `body`, `size`, `redirect_chain`, `retrieved_at`.

`final_url` **MAY** existir transitoriamente en `FetchResult` (salida del Fetcher); sólo `Document` **MUST** persistirla/exponerla como dato de Deep. `final_url` existe sólo después de Fetch/Deep.

Reglas: validar SSRF antes de conectar, validar tras resolución DNS cuando aplique, validar cada redirect, limitar bytes, limitar redirects, permitir sólo esquemas autorizados, no ejecutar contenido.

### 7.19 Renderer

Responsabilidad: renderizar páginas que requieren JavaScript.

- Motor: **Chromium headless** (vía CDP).
- **MUST** ser opcional, aislado, sandbox, con timeout, límite de memoria y navegación limitada, sin acceso a red interna.
- **MUST NOT** formar parte del binario ni de la instalación base.
- Se detecta en runtime; si no existe, Deep continúa sin render.
- Excluido de Search; sujeto a Budget; se introduce en Fase 5+.

### 7.20 Extractor

Responsabilidad: convertir HTML/DOM en contenido principal y metadata.

Backends: Trafilatura, Native futuro, Alternative.

Salida: `content`, `format`, `title`, `author`, `published_at`, `metadata`, `extractor_used`, `status`.

Reglas:

- no hace networking;
- no renderiza JavaScript;
- no decide ranking;
- Trafilatura es capability opcional y reemplazable; el binario base **MUST NOT** depender de Python; si Trafilatura no está instalado, Deep degrada sin extracción avanzada.

### 7.21 Document

Cada Document conserva: `search_result_id`, `original_url`, `canonical_url`, `final_url`, `content_hash`, `fetch_method`, `extractor_used`, `content_type`, `size`, `retrieved_at`, `status`, `schema_version`.

El cuerpo completo pertenece a Document, no a SearchResult.

### 7.22 Evidence (implementado)

Entidad exclusiva de Deep. Representa señales derivadas de Document para Ranking v2 y Gap Analyzer. Puede incluir: fact density, verified date, metadata quality, named entities, citation span, freshness, originality. **MUST NOT** formar parte del Search MVP.

### 7.23 Ranking v2

Sólo sobre candidatos Deep. Puede utilizar: BM25, embeddings, reranking, `evidence_score`. `evidence_score` permanece separado del score de relevancia. **MUST NOT** incorporarse sin benchmark frente a Ranking MVP.

### 7.24 Gap Analyzer (implementado)

Responsabilidad: detectar déficits observables. No ejecuta búsquedas.

Salida: `gap_type`, `severity`, `reason`, `recommended_query`, `estimated_cost`, `expected_gain`.

Tipos: `primary_source`, `recency`, `geographic_diversity`, `documentation`, `pdf`, `code`, `specification`, `source_diversity`.

Sólo Deep Orchestrator puede ejecutar SubQuery.

### 7.25 Query Expansion

Máximo inicial: 0–2 variantes. Sólo cuando Gap lo justifique, la cobertura sea baja y la consulta sea difícil. **MUST NOT** existir loops agentic abiertos.

---

## 8. Provider Governance

Evaluación por provider individual; no es política global.

- Cada fuente **MUST** tener documentada su situación de términos de servicio (ToS) y coste antes de ser activada.
- El uso de scraping HTML se decide caso a caso.
- **MUST NOT** añadirse ningún provider sin nota explícita de ToS/coste.

Clasificación por nivel de servicio:

- `stable` (base): **Brave Search API** y **Mojeek** — columna vertebral de la Fase 1.
- `best_effort`: **DuckDuckGo (HTML)** — experimental; su caída **MUST NOT** afectar el estado global si existen resultados de otros providers.

---

## 9. Configuración y versionado

### 9.1 Configuración

Archivo: `amatl.toml`. Esquema **mínimo incremental**:

- Fase 0: `providers`, `timeouts`, `budget`.
- Fases posteriores: idioma, región, caché, routing, Deep, UI, HTTP exposure, ranking policy.

El esquema crece por fases sin bloquear el arranque. **MUST NOT** contener secretos versionados; los secretos van en variables de entorno.

### 9.2 Versionado

- `schema_version`: contrato de datos expuestos. Tipo **string**; valor inicial `"1"`. **MUST NOT** confundirse con el SemVer del binario. Operativo: dentro de v1 sólo **cambios aditivos compatibles**; cualquier ruptura contractual **MUST** incrementar `schema_version`.
- `adapter version` / `extractor version`: versiones de componente usadas en claves de caché.
- Políticas versionadas: `ranking_policy` (v1) y umbrales de Diversity.
- SQLite usa **migraciones independientes y versionadas**, separadas del schema de contratos.

---

## 10. Persistencia, caché y telemetría

### 10.1 SQLite

Configuración: `journal_mode = WAL`, `busy_timeout = 5000`, `synchronous = NORMAL`. Pool pequeño inicialmente, ajustado por benchmark, no relacionado automáticamente con CPU count. SQLite **MUST NOT** formar parte de correctness.

### 10.2 Cache

**ProviderSearchCache** (cuando la clave incluye `provider`): clave = normalized query, filtros, provider, adapter version. Opera **antes** del pipeline global de dedupe/ranking y **MUST NOT** mezclarse con la caché del `SearchResult` final.

Document cache key: canonical URL, content hash, extractor version.

Políticas: TTL, tamaño máximo, antigüedad, LRU, invalidación por versión.

Reglas: cache descartable; fallo de cache **MUST NOT** romper Search; fallo de SQLite **MUST NOT** romper Search; escrituras no críticas tolerantes a fallo.

### 10.3 Telemetría

- Métricas **MUST** vivir **en memoria durante la ejecución**; SQLite sólo persiste de forma **opcional**.
- Si SQLite falla, el routing sigue funcionando con métricas en memoria.
- Al reiniciar sin persistencia, el sistema vuelve a `Bootstrap`; no hay degradación silenciosa a routing estático sin métricas vivas.

Métricas Search: latency, success, error, timeout, total results, unique results, duplicate ratio, top-K contribution, cost.

Deep: fetch success, extraction success, browser fallback, documents enriched, gaps detected, subqueries.

Seguridad: blocked URL, blocked redirect, oversized response, rate limit, invalid host.

**MUST NOT** almacenar por defecto: tokens, passwords, auth headers, cookies, secretos, contenido completo fuera de caché documental explícita.

**MUST** existir retención, ventana temporal y decaimiento de datos.

---

## 11. Degradación

Estados globales: `success`, `partial_success`, `failure`. `partial_success` **MUST** tratarse como estado normal del sistema.

| Componente | Fallo | Acción |
|---|---|---|
| Classification | No clasifica | `general` |
| Router | Sin telemetría | `Bootstrap` |
| Provider | Timeout | Continuar |
| Provider | Rate limit | Continuar |
| Provider | Parcial | Conservar |
| Normalization | Campo corrupto | Degradar campo |
| Canonicalization | Parcial | Conservar original |
| Deduplication | Incertidumbre | `possible_duplicate` |
| Ranking | Señal ausente | Usar restantes |
| Diversity | No aplicable | Mantener ranking |
| SQLite | Falla | Continuar |
| Cache | Falla | Continuar |
| Telemetry | Falla | Continuar salvo evento crítico |
| Fetcher | Falla | Conservar SearchResult |
| Trafilatura | Falla | Conservar superficial |
| Renderer | Falla | Continuar sin render |
| Gap Analyzer | Falla | Finalizar Deep |
| Budget | Agotado | Resultado parcial |
| SSRF | Bloqueo | No bypass |
| Todos providers | Fallan | Error global compuesto |

---

## 12. Límites operativos

Los valores de esta sección son **benchmark-calibrated**: se ajustan empíricamente **sin** cambiar contratos ni `schema_version`.

Search:

- timeout provider: 3–8 s;
- retry normal: 1; retry máximo: 2;
- concurrencia limitada;
- resultados máximos por provider;
- providers máximos por ronda.

Retries **MUST** respetar `Retry-After`, backoff exponencial y jitter.

Deep:

- crawl depth normal: 1; máximo inicial: 2;
- fetches máximos;
- bytes máximos;
- browser calls máximas;
- subqueries máximas.

Timeouts, concurrencia, pool SQLite, resultados por provider y thresholds **MAY** ajustarse empíricamente.

---

## 13. Seguridad

### 13.1 Base

Secretos fuera del código y del repositorio; tokens fuera de logs; variables de entorno para secretos; inputs validados; URLs parseadas estructuralmente; timeouts obligatorios; límites HTTP; redirects limitados; rate limiting; privilegios mínimos; dependencias mínimas; `Cargo.lock` versionado; Cargo Audit; Cargo Deny; SBOM.

### 13.2 Seguridad de red

Bloquear o restringir: localhost, loopback, redes privadas, link-local, endpoints internos, esquemas no permitidos.

Validación: 1) antes de conectar, 2) tras DNS cuando corresponda, 3) después de cada redirect.

Objetivo: prevenir SSRF, evitar acceso a infraestructura interna, proteger API y MCP futuros.

### 13.3 Ejecución de contenido remoto

El core **MUST NOT** ejecutar scripts, binarios o contenido descargado. JavaScript sólo dentro del Renderer aislado.

### 13.4 HTTP Security / Web Hardening

CSP baseline: `default-src 'self'`, `object-src 'none'`, `base-uri 'self'`, `frame-ancestors 'none'`. Definir explícitamente `script-src`, `style-src`, `img-src`, `connect-src`. Evitar `unsafe-inline` y `unsafe-eval` salvo justificación.

Headers: `Content-Security-Policy`, `X-Content-Type-Options`, `Referrer-Policy`, `Permissions-Policy`, `Strict-Transport-Security` con HTTPS, `Cache-Control` adecuado.

CORS restrictivo por defecto; sin wildcard en superficies sensibles. Host/Origin con validación explícita.

Request limits: body size, header size, request/response/idle timeout, conexiones.

Rate limiting por IP, endpoint y token.

Cookies (si existen sesiones): `Secure`, `HttpOnly`, `SameSite`.

No exponer información innecesaria sobre framework, versión o infraestructura.

### 13.5 Exposición HTTP y autenticación

Default bind: `127.0.0.1`. Exposición externa `0.0.0.0` sólo mediante configuración explícita.

- CLI local: **sin autenticación**.
- Servidor HTTP/MCP: **token local desde el inicio**.
- `no-auth` sólo como modo explícito de desarrollo.
- Exposición remota (`0.0.0.0`): **autenticación + TLS obligatorios**.
- Acceso remoto **MUST** incluir TLS, autenticación cuando corresponda, CORS, Host validation, rate limiting y hardening completo.

---

## 14. Observabilidad

Niveles: `ERROR`, `WARN`, `INFO`, `DEBUG`, `TRACE`.

Formato de logs:

- JSON estructurado en stderr al redirigir/pipe;
- humano en tty (auto-detección de terminal);
- campos estables mínimos: `ts`, `level`, `target`, `msg`, contexto.

Modo normal: mínimo ruido. Debug: Classification, SearchPlan, routing, Budget, provider decisions, timings, retries, canonicalization, dedupe, ranking, cache, degradations.

---

## 15. Interfaces

### 15.1 CLI

Comandos MVP: `amatl search`, `amatl providers`, `amatl config`, `amatl cache`, `amatl doctor`. Post-MVP (Fase 5+): `amatl deep`. Posteriormente: `amatl mcp serve`.

Salida:

- humana por defecto (legible, orientada a `buscar → revisar → abrir`);
- `--json` para salida estructurada;
- códigos de salida: `0` éxito, `1` error, `2` uso incorrecto;
- `partial_success` **MUST** devolver código `0` (estado normal con resultados útiles); la diferencia frente a `success` se conserva en `status` (JSON) y en la salida humana (marcado distinguible).

`amatl doctor`: diagnóstico completo del sistema local (config, providers, caché, telemetría). `/health` (API): chequeo ligero de disponibilidad. Distinción semántica documentada.

### 15.2 API (futura)

Axum. Endpoints conceptuales: `/search`, `/deep`, `/providers`, `/health`. La API **MUST** consumir tipos y contratos del core; **MUST NOT** tener lógica duplicada.

### 15.3 MCP (futuro)

Funciones previstas: `search`, `deep_search`, `fetch`, `providers`. Consume los mismos contratos, Budget y reglas de seguridad. Los límites de MCP **MUST** ser más restrictivos que CLI local para operaciones costosas.

### 15.4 UI

Características: minimalista, responsiva, rápida, orientada a resultados. Flujo `buscar → revisar → abrir`.

Mostrar: campo de búsqueda, filtros básicos, resultados, navegación, carga.

**MUST NOT** mostrar por defecto: dashboards, RRF, scores, latencias, retries, provider health, telemetría, bloques extensos.

Diseño responsivo: desktop (búsqueda superior/central, columna principal, filtros discretos), tablet (layout fluido, filtros colapsables), móvil (una columna, controles táctiles, snippets reducidos, filtros plegables, sin scroll horizontal). Layout con `rem`, `%`, `clamp()`, Flexbox, Grid, pocos breakpoints.

Tipografía: `Inter` (búsqueda, resultados, navegación, botones, filtros); `JetBrains Mono` (URLs, providers, logs, comandos, identificadores). Tamaños: título 26–28 px, encabezado 20–22 px, resultado 17–18 px, snippet 15–16 px, metadata 12–13 px, monospace 13–14 px, mínimo 12 px.

Paleta: fondo `#111315`, superficie `#181B1F`, borde `#2A2F35`, texto principal `#E7E9EC`, secundario `#9DA5AE`, tenue `#6F7780`; acentos azul `#4F8CFF`, cian opcional `#48B8C7`; éxito `#4FAE72`, advertencia `#D6A84B`, error `#D95C5C`. Uso semántico, no decorativo.

Resultado visual: título, dominio o URL, snippet breve, fecha cuando exista, provider cuando aporte valor. Detalles técnicos sólo en debug.

### 15.5 Formato JSON

Respuesta base:

```json
{
  "schema_version": "1",
  "query": "...",
  "status": "success",
  "results": [],
  "providers_used": [],
  "providers_failed": [],
  "providers_partial": [],
  "elapsed_ms": 0
}
```

Resultado:

```json
{
  "rank": 1,
  "title": "...",
  "original_url": "...",
  "canonical_url": "...",
  "domain": "...",
  "snippet": "...",
  "providers": [],
  "published_at": null,
  "status": "visible"
}
```

En Search **MUST** exponerse únicamente `original_url` y `canonical_url`. `final_url` queda reservado para Deep/Document y **MUST NOT** aparecer en la salida de Search. La UI **MAY** derivar una URL de presentación desde `canonical_url`; no es un campo del contrato.

---

## 16. Testing

- Unit: Query parser, Classification, SearchPlan, Budget, canonicalization, normalization, dedupe, ranking, diversity, adapters, URL security.
- Integration: providers, parallel search, SQLite, cache, Deep, Trafilatura, Renderer, CLI, API, MCP.
- Property: URL parsing, canonicalization, Query parser, dedupe.
- Security: SSRF, DNS rebinding, redirects, private ranges, malformed URLs, oversized responses/headers, CORS, CSP, Host validation, rate limiting.
- Contract: cada frontera **MUST** cubrir entrada válida, entrada degradada, error tipado, resultado parcial, invariantes y Budget agotado.

Los contract tests **MUST** ser **requisito de merge por módulo**. Ninguno de estos módulos se considera terminado sin sus pruebas contractuales: provider, canonicalization, deduplication, Budget, ranking, Fetcher, extractor, router, normalization.

---

## 17. Benchmarks y métricas

Benchmarks (medir): latencia total, provider latency, throughput, memoria, Tokio concurrency, SQLite contention, dedupe, RRF, routing, marginal gain, extraction, Renderer fallback, Deep latency.

Métricas operativas Search: unique useful results, unique domains, duplicate ratio, provider contribution, top-K quality, latency, error rate, marginal gain, cost/query, partial_success rate.

Métricas operativas Deep: fetch success, extraction success, enriched results, Gap detection, SubQuery utility, reranking improvement, browser fallback rate.

---

## 18. Fases de implementación

**Fase 0 — Contratos.** Cerrar y contract-testear: ciclo de vida de entidades, clasificación (heurística léxica), normalización degradada, Provider Value (estados, inputs, outputs e invariantes), Gap Analyzer (sólo stub/interfaz/frontera), Budget, ranking/diversidad, degradación. Además: `schema_version`, semántica `error/warning/degradation`, ownership del Budget. La telemetría real, la calibración y el comportamiento adaptativo de Provider Value se implementan en Fases 3–4; la implementación y el comportamiento funcional de Gap Analyzer permanecen en Fase 7.

**Fase 1 — Core MVP.** Rust, Tokio, Reqwest, Serde, Clap, Query, Classification, SearchPlan, Provider interface, Budget, Parallel Search. Providers Fase 1: **Brave Search API** y **Mojeek** (`stable`), **DuckDuckGo HTML** (`best_effort`).

**Fase 2 — Result Pipeline.** Normalization, Canonicalization, Deduplication, RRF, Diversity, JSON, CLI. Objetivo: `amatl search`.

**Fase 3 — Persistencia y Telemetría.** SQLite WAL, ProviderSearchCache, Telemetry, provider health, estados Bootstrap/Learning/Mature.

**Fase 4 — Routing adaptativo.** Provider Value, progressive search, marginal gain, stop conditions, exploration mínima.

**Fase 5 — Deep.** Fetcher, SSRF controls, Trafilatura (opcional), Document Cache, Renderer opcional (Chromium runtime), crawl limitado.

**Fase 6 — Ranking v2.** Sólo tras benchmark: BM25, embeddings opcionales, reranking, Evidence, evidence_score.

**Fase 7 — Gap Analyzer.** Gaps observables, SubQuery, expected gain, Budget integration.

**Fase 8 — UI.** Interfaz responsiva, tipografías, paleta, layout, CSP, hardening.

**Fase 9 — API / MCP.** Axum, API, MCP, CORS, Host validation, rate limiting, token local, TLS cuando corresponda.

**AMATL v1 / MVP = Fases 0–4.** El fin de Fase 2 es el *Search functional milestone* (no el MVP completo). Deep (Fase 5+) comienza después del MVP.

---

## 19. Criterios de aceptación MVP

AMATL v1 **MUST**:

1. recibir Query;
2. interpretar operadores;
3. producir Classification;
4. generar SearchPlan;
5. asignar Budget;
6. seleccionar providers;
7. consultar providers concurrentemente;
8. sobrevivir a fallos parciales;
9. normalizar resultados;
10. canonicalizar URLs;
11. deduplicar conservando procedencia;
12. fusionar rankings mediante RRF;
13. aplicar Diversity;
14. devolver `success`, `partial_success` o `failure`;
15. registrar telemetría básica;
16. aplicar stop conditions;
17. continuar sin cache;
18. continuar sin SQLite;
19. funcionar sin LLM;
20. proteger secretos;
21. aplicar límites de red;
22. funcionar nativamente en Linux.
