# AMATL — Fase A: contratos ejecutables

Documento de trabajo técnico no normativo para la Fase 0. No sustituye, amplía
ni modifica `plan_amatl.md`, que permanece como la única fuente normativa.
Toda regla de este documento que no esté ya exigida por el golden template es
una propuesta pendiente de promoción explícita al golden; no autoriza por sí
misma una implementación incompatible con éste.

## 1. Propósito y límites

Este documento cierra la forma de los contratos que cruzan fronteras de módulo,
persistencia o interfaz. No incorpora adapters, networking, SQLite, Deep ni
políticas adaptativas.

Regla general de compatibilidad para `schema_version = "1"`:

- los cambios de contrato sólo pueden ser aditivos;
- un consumidor debe ignorar campos desconocidos;
- un productor no debe cambiar el significado, tipo ni nulabilidad de un campo
  existente;
- una ruptura exige un incremento de `schema_version`.

## 2. Convenciones de tipos y JSON

| Concepto | Rust | JSON | Regla |
|---|---|---|---|
| Identificador de esquema | `String` | string | Valor inicial obligatorio: `"1"`. |
| Texto requerido | `String` | string | No admite `null`. Puede estar vacío sólo si el contrato lo permite explícitamente. |
| Texto opcional | `Option<String>` | string o `null` | Se serializa como `null`; no se omite. |
| Colección | `Vec<T>` | array | Requerida; vacía se representa como `[]`. |
| Mapa | `BTreeMap<K, V>` | object | Requerido; vacío se representa como `{}`. Se prefiere orden estable. |
| Instante | `DateTime<Utc>` | RFC 3339 UTC | Obligatorio sólo cuando el contrato lo declare. |
| Instante opcional | `Option<DateTime<Utc>>` | RFC 3339 UTC o `null` | Fecha inválida se convierte en `null` y conserva el original en metadata si existe. |
| URL | newtype validado | string | Se expone como string; se valida antes de cruzar la frontera correspondiente. |
| Enum | `#[serde(rename_all = "snake_case")]` | string snake_case | Los valores no reconocidos son error de contrato salvo `result_type`, que admite `other`. |

Los números no usan `NaN` ni infinito. Scores y confidencias se rechazan fuera
del intervalo `[0.0, 1.0]`. Los campos de depuración no forman parte de la
respuesta base salvo una superficie explícita de debug futura.

## 3. Tipos de valor canónicos

Los siguientes tipos son newtypes en Rust para impedir intercambios accidentales
entre valores conceptualmente distintos:

```rust
pub struct OriginalUrl(pub Url);
pub struct CanonicalUrl(pub Url);
pub struct FinalUrl(pub Url);
pub struct Rank(pub u32);              // invariante: >= 1
pub struct RankingScore(pub f64);      // invariante: 0.0..=1.0
pub struct SchemaVersion(pub String);  // inicial: "1"
```

Enums cerrados requeridos:

```text
SearchStatus: success | partial_success | failure
ResultStatus: visible | relegated_by_diversity
ProviderErrorKind: timeout | rate_limit | auth | network | invalid_response |
                   parser_error | quota | unavailable
ProviderValueState: bootstrap | learning | mature
DuplicateStatus: confirmed_duplicate | possible_duplicate | distinct
```

`ProviderError` es una estructura y no sólo un enum: contiene `kind`,
`provider`, un mensaje seguro para diagnóstico y, cuando aplique, información
de reintento no sensible. Nunca contiene tokens, cookies, headers ni secretos.

## 4. Catálogo de contratos

### 4.1 Query

```text
Query {
  schema_version: String,
  raw_query: String,
  normalized_query: String,
  quoted_terms: Vec<String>,
  excluded_terms: Vec<String>,
  domains: Vec<String>,
  excluded_domains: Vec<String>,
  file_types: Vec<String>,
  language: Option<String>,
  region: Option<String>,
  date_from: Option<DateTime<Utc>>,
  date_to: Option<DateTime<Utc>>,
  warnings: Vec<QueryWarning>
}
```

Invariantes:

- `raw_query` se preserva byte a byte desde la entrada del usuario.
- `normalized_query` no contiene operadores reconocidos ni filtros extraídos.
- `date_from <= date_to` si ambos valores existen.
- listas sin elementos son `[]`; filtros no especificados singulares son `null`.
- un warning no impide construir una Query consumible.

`QueryWarning`:

```text
{ code: String, operator: Option<String>, value: Option<String>, message: String }
```

### 4.2 Classification

```text
Classification {
  schema_version: String,
  primary_category: Category,
  secondary_categories: Vec<Category>,
  confidence: f64,
  confidence_by_category: BTreeMap<Category, f64>,
  reasons: Vec<String>
}
```

`Category` es: `general`, `technical`, `code`, `documentation`, `news`,
`academic`, `commercial`, `forum`, `social`, `media`, `navigation`.

Invariantes:

- existe exactamente una categoría primaria;
- hay como máximo dos secundarias, sin duplicados ni repetición de la primaria;
- `confidence` es idéntica a la entrada de la primaria en
  `confidence_by_category`;
- ante falta de evidencia: primaria `general`, confianza determinista y
  `reasons` no vacío.

### 4.3 ProviderCapabilities y ProviderError

```text
ProviderCapabilities {
  schema_version: String,
  pagination: bool, language: bool, region: bool, time_range: bool,
  site_filter: bool, file_filter: bool, news: bool, code: bool, docs: bool,
  academic: bool, authentication: bool,
  estimated_cost: Option<CostEstimate>
}

ProviderError {
  schema_version: String,
  provider: String,
  kind: ProviderErrorKind,
  message: String,
  retry_after_ms: Option<u64>
}
```

`CostEstimate` expresa una estimación documentada por solicitud, con moneda o
unidad explícita. `retry_after_ms` sólo se rellena si procede de una señal
verificable del provider; no se inventa.

### 4.4 ProviderResult

```text
ProviderResult {
  schema_version: String,
  provider: String,
  adapter_version: String,
  status: ProviderExecutionStatus,
  results: Vec<ProviderItem>,
  accepted_filters: Vec<String>,
  ignored_filters: Vec<String>,
  approximated_filters: Vec<String>,
  errors: Vec<ProviderError>
}

ProviderItem {
  title: Option<String>,
  url: String,
  provider_rank: Option<Rank>,
  snippet: Option<String>,
  published_at: Option<DateTime<Utc>>,
  author: Option<String>,
  language: Option<String>,
  file_type: Option<String>,
  thumbnail: Option<String>,
  metadata: BTreeMap<String, Value>,
  result_type: Option<ResultType>
}
```

`ProviderExecutionStatus` es `success`, `partial` o `failure`. Un resultado
parcial conserva sus items válidos y sus errores. `provider_rank` sólo existe
si el adapter preserva un orden nativo verificable.

### 4.5 NormalizedResult y CanonicalResult

```text
NormalizedResult {
  schema_version: String,
  title: Option<String>,
  url: OriginalUrl,
  provider: String,
  result_type: ResultType,
  provider_rank: Option<Rank>,
  snippet: Option<String>,
  published_at: Option<DateTime<Utc>>,
  author: Option<String>, language: Option<String>, file_type: Option<String>,
  thumbnail: Option<String>, metadata: BTreeMap<String, Value>,
  degradations: Vec<Degradation>
}

CanonicalResult {
  schema_version: String,
  normalized: NormalizedResult,
  original_url: OriginalUrl,
  canonical_url: CanonicalUrl,
  transformations: Vec<CanonicalTransformation>,
  canonicalization_status: CanonicalizationStatus
}
```

`ResultType`: `organic`, `news`, `media`, `document`, `code`, `forum`,
`social`, `commercial`, `navigation`, `other`. Ausencia en provider se
normaliza a `organic`; un tipo no clasificable se normaliza a `other`.

`CanonicalizationStatus`: `complete` o `degraded`. Una URL inválida no produce
`NormalizedResult`; se descarta antes y queda registrada como degradación del
provider/pipeline.

### 4.6 DeduplicatedResult y SearchResult

```text
DeduplicatedResult {
  schema_version: String,
  duplicate_status: DuplicateStatus,
  representative: CanonicalResult,
  providers: Vec<String>,
  provider_ranks: BTreeMap<String, Option<Rank>>,
  original_urls: Vec<OriginalUrl>,
  alternative_snippets: Vec<String>,
  observed_dates: Vec<DateTime<Utc>>,
  merge_reasons: Vec<MergeReason>
}

SearchResult {
  schema_version: String,
  rank: Rank,
  title: Option<String>,
  original_url: OriginalUrl,
  canonical_url: CanonicalUrl,
  domain: String,
  snippet: Option<String>,
  providers: Vec<String>,
  published_at: Option<DateTime<Utc>>,
  status: ResultStatus
}
```

`SearchResult` no contiene `final_url`, cuerpo, score interno, RRF ni datos de
Deep. `domain` se deriva exclusivamente de `canonical_url`.

### 4.7 SearchPlan y Budget

```text
SearchPlan {
  schema_version: String,
  query: Query,
  classification: Classification,
  ranking_reference_time: DateTime<Utc>,
  selected_providers: Vec<String>,
  provider_priority: Vec<String>,
  provider_budget_requests: BTreeMap<String, BudgetRequest>,
  provider_budgets: BTreeMap<String, ProviderBudgetSnapshot>,
  global_budget: GlobalBudgetSnapshot,
  fallback_policy: String,
  expansion_policy: String,
  stop_conditions: Vec<String>,
  debug_reasons: Vec<String>
}
```

`provider_budget_requests` pertenece a routing. `provider_budgets` y
`global_budget` son snapshots producidos sólo por el orquestador. Ningún
provider puede mutar el plan. `ranking_reference_time` se captura una sola vez
al crear el plan y se conserva en retries, rondas, cache y reconstrucciones de
debug para que las señales temporales sean reproducibles.

```text
BudgetRequest { requested_time_ms, requested_cost, requested_results }
ProviderBudgetSnapshot { reserved_time_ms, reserved_cost, max_results }
GlobalBudgetSnapshot { deadline_at, max_cost, max_providers, max_subqueries }
BudgetExhaustion { cause: BudgetExhaustionCause }
```

Las causas son las canónicas de §7.7 del golden template. Los valores no
aplicables se expresan como `null`, no como cero; cero significa límite activo
sin capacidad disponible.

### 4.8 Respuesta pública de Search

```text
SearchResponse {
  schema_version: String,
  query: String,
  status: SearchStatus,
  results: Vec<SearchResult>,
  providers_used: Vec<String>,
  providers_failed: Vec<String>,
  providers_partial: Vec<String>,
  elapsed_ms: u64
}
```

Invariantes:

- `success` no contiene fallos ni parcialidad de provider;
- `partial_success` tiene al menos un resultado útil y uno o más fallos,
  degradaciones o límites agotados;
- `failure` no tiene resultados útiles y debe acompañarse de un error compuesto
  en la frontera que lo transporte;
- `rank` es consecutivo, inicia en 1 y sigue el orden de `results`;
- los tres vectores de providers no se solapan.

## 5. Semántica transversal de incidentes

```text
Error       = la operación o frontera no produjo el artefacto requerido.
Warning     = la operación continuó, pero la intención del usuario fue ambigua
              o un input se reinterpretó de forma explícita.
Degradation = la operación continuó con menor fidelidad, cobertura o capacidad.
```

Los tres conceptos se modelan con estructuras tipadas separadas y códigos
estables en `snake_case`. No se convierten entre sí de forma implícita.

## 6. Fixtures normativos iniciales

Los siguientes fixtures deben vivir más adelante en tests de contrato; se
definen ahora como casos obligatorios.

| ID | Frontera | Caso | Resultado esperado |
|---|---|---|---|
| Q-01 | Query | Texto sin operadores | Query válida, listas vacías, sin warnings. |
| Q-02 | Query | Operador con valor inválido | Query consumible y `QueryWarning` tipado. |
| C-01 | Classification | Sin señales léxicas | `general`, salida determinista. |
| P-01 | Provider | Resultado parcial y rate limit | Items válidos preservados; error seguro y estado `partial`. |
| N-01 | Normalization | URL inválida | Item descartado; resto del provider se conserva. |
| N-02 | Normalization | Fecha inválida | `published_at: null`; valor original permitido en metadata. |
| CA-01 | Canonicalization | `utm_*` conocido | Se elimina y se registra transformación. |
| CA-02 | Canonicalization | Parámetro ambiguo `ref` | Se conserva. |
| D-01 | Deduplication | Misma canonical URL | `confirmed_duplicate`, procedencia preservada. |
| D-02 | Deduplication | Sólo título similar | Nunca fusión automática; `possible_duplicate` o `distinct`. |
| B-01 | Budget | Reserva agotada antes de iniciar provider | Provider no se invoca; causa tipada. |
| B-02 | Budget | Timeout con resultados existentes | Salida Search `partial_success`. |
| S-01 | SearchResponse | Resultado completo | JSON exacto conforme a §15.5. |
| S-02 | SearchResponse | Un provider falla, hay resultados útiles | Exit code 0 y `partial_success`. |
| S-03 | SearchResponse | Todos los providers fallan | `failure`, sin resultados, error compuesto. |

## 7. Checklist de cierre de Fase A

- [ ] Cada tipo de valor tiene representación Rust, reglas de validación y JSON.
- [ ] Cada enum tiene valores canónicos y serialización `snake_case`.
- [ ] Campos opcionales usan `null`; colecciones vacías usan `[]` o `{}`.
- [ ] Toda estructura persistida o expuesta lleva `schema_version`.
- [ ] No existen secretos en errores, warnings, degradations ni fixtures.
- [ ] Las invariantes de `SearchResponse` son comprobables por test.
- [ ] Existen fixtures para ruta válida, degradada, error tipado, parcial y
      Budget agotado en cada frontera prioritaria.
- [ ] El catálogo ha sido revisado contra §§5–7, 9, 11, 15 y 16 del golden
      template.

---

# AMATL — Fase B: semántica transaccional de Budget

Documento complementario de diseño. Esta fase no cambia la propiedad exclusiva
del Budget por el orquestador, definida en el golden template.

## 8. Objetivo y alcance

El Budget debe impedir que operaciones concurrentes excedan límites globales o
por recurso, aun cuando se cancelen, fallen o completen fuera de orden. Aplica a
Search y, posteriormente, a Deep. No convierte al router, providers, Fetcher,
Renderer ni Extractor en propietarios de límites.

El modelo es de **reserva conservadora + liquidación de consumo real**. Sólo
`SearchOrchestrator` y `DeepOrchestrator` pueden crear reservas, iniciar su
ejecución y liquidarlas.

## 9. Modelo de datos propuesto

```text
Budget {
  budget_id: UUID,
  stage: BudgetStage,
  deadline_at: Instant,
  limits: BudgetLimits,
  committed: BudgetUsage,
  reserved: BudgetUsage,
  state: BudgetState
}

BudgetLimits {
  max_cost: Option<CostUnits>,
  max_providers: Option<u32>,
  max_subqueries: Option<u32>,
  max_bytes: Option<u64>,
  max_redirects: Option<u32>,
  max_browser_calls: Option<u32>,
  max_crawl_urls: Option<u32>
}

BudgetUsage {
  cost: CostUnits,
  providers: u32,
  subqueries: u32,
  bytes: u64,
  redirects: u32,
  browser_calls: u32,
  crawl_urls: u32
}

Reservation {
  reservation_id: UUID,
  budget_id: UUID,
  owner: ReservationOwner,
  granted: BudgetUsage,
  consumed: BudgetUsage,
  created_at: Instant,
  state: ReservationState
}
```

`BudgetStage` es `search`, `fetch`, `render`, `extract`, `crawl` o
`gap_expansion`. `ReservationOwner` identifica una operación concreta, por
ejemplo `provider:brave:round:1`; no contiene secretos. `CostUnits` usa una
unidad interna entera de coste para evitar errores de coma flotante.

Estados:

```text
BudgetState: active | exhausted | expired | closed
ReservationState: reserved | running | settled | cancelled | rejected
```

Una reserva es inmutable respecto a su límite concedido. Sólo el orquestador
puede crear otra reserva; no hay ampliación ni transferencia implícita entre
owners.

## 10. Invariantes transaccionales

1. Para cada dimensión limitada: `committed + reserved <= limits`.
2. Una reserva sólo puede pasar de `reserved` a `running` una vez.
3. `consumed <= granted` para cada dimensión de una reserva.
4. Una reserva `settled`, `cancelled` o `rejected` es terminal e inmutable.
5. La liquidación sólo puede ejecutarse una vez por `reservation_id`.
6. Al liquidar, el remanente se libera y únicamente el consumo real pasa a
   `committed`.
7. Si no se puede medir un consumo antes de realizar una acción limitada, se
   reserva el máximo permitido para esa acción.
8. El vencimiento de `deadline_at` impide nuevas reservas e inicia cancelación
   cooperativa de las operaciones en curso.
9. Ningún módulo consumidor puede crear, ampliar, reutilizar ni transferir una
   reserva.
10. Toda decisión de rechazo o agotamiento incluye una causa canónica de §7.7.

## 11. Ciclo de vida de una reserva

```text
request
  → validate deadline and limits
  → atomically reserve capacity
  → reserved
  → start operation
  → running
  → report bounded consumption
  → settle once
  → settled

reserved/running → cancellation or deadline → cancelled → settle once
request → insufficient capacity/deadline expired → rejected
```

La operación externa se inicia sólo después de que la reserva haya sido
registrada atómicamente. Si falla antes de comenzar, se liquida con consumo cero
y se libera toda la reserva. Si existe un coste externo no recuperable, dicho
coste se registra como consumo incluso si la respuesta termina en error.

## 12. Operaciones atómicas y concurrencia Tokio

La mutación de estado se centraliza en un único componente interno del
orquestador, por ejemplo `BudgetLedger`. Los providers reciben un handle de
lectura/consumo limitado asociado a su `reservation_id`, no un `Budget` mutable.

Operaciones requeridas:

```text
reserve(request) -> Reservation | BudgetExhaustion
start(reservation_id) -> RunningReservation | InvalidReservationState
record_usage(reservation_id, delta) -> Ok | ReservationLimitExceeded
settle(reservation_id) -> Settlement
cancel(reservation_id, cause) -> Settlement
remaining() -> BudgetRemaining
```

`reserve`, `record_usage`, `settle` y `cancel` deben ser linealizables: cada
operación observa un orden único de confirmación. La implementación puede usar
un mutex asíncrono de corta duración o un actor de ledger; no debe mantener el
lock mientras se realiza red, parsing, backoff ni cancelación de tareas.

`record_usage` rechaza deltas que excedan `granted - consumed`; el consumidor
debe detener la operación antes de superar su propio límite. Para bytes y
redirects, el consumidor comprueba el límite antes de aceptar el siguiente
chunk o redirect.

## 13. Semántica por tipo de consumo

| Recurso | Momento de reserva | Consumo real | Cancelación |
|---|---|---|---|
| Provider | Antes de lanzar su tarea | 1 cuando se inicia la consulta | No libera el slot ya iniciado |
| Coste | Antes de acción facturable | Coste conocido o reserva conservadora | Libera sólo coste no incurrido |
| Tiempo | Deadline global, no saldo transferible | Tiempo observado para telemetría | Deadline cancela; no se "devuelve" tiempo |
| Bytes | Antes de lectura HTTP | Bytes aceptados | Libera bytes no descargados |
| Redirects | Antes de seguir redirect | Redirect confirmado | Libera redirects no usados |
| Browser call | Antes de iniciar Chromium | 1 al iniciar proceso/navegación | No libera llamada iniciada |
| Crawl URL | Antes de encolar URL | 1 al iniciar fetch | Libera URL no iniciada |
| Subquery | Antes de planificar ejecución | 1 al ejecutar | Libera sólo si nunca se lanzó |

La concurrencia no es un recurso contable del Budget; es un límite de ejecución
del orquestador. No obstante, la tarea sólo puede entrar a la cola concurrente
si dispone de reserva válida.

## 14. Deadline, timeout y cancelación

- `deadline_at` es un límite duro global definido por el orquestador.
- El timeout individual del provider nunca puede superar el tiempo restante al
  deadline menos una reserva de cierre configurable.
- Al aproximarse el deadline, el orquestador deja de crear reservas nuevas con
  causa `deadline_near`.
- Al vencer, cancela tareas en curso de manera cooperativa y liquida cada
  reserva una vez.
- Un resultado ya recibido antes de la cancelación permanece válido y puede
  producir `partial_success`.
- Retry usa la misma reserva o una reserva explícita nueva; nunca duplica el
  cupo de provider ni sobrepasa coste/tiempo disponibles. El backoff no extiende
  el deadline.

## 15. Reglas de resultado y observabilidad

Cada liquidación genera un evento estructurado seguro:

```text
budget_id, reservation_id, owner, state, granted, consumed,
released, exhaustion_cause, elapsed_ms
```

No se registran credenciales, headers, contenido ni URLs completas si la
política de logs no lo permite. El usuario sólo recibe una degradación o estado
global pertinente; los detalles de reserva permanecen en debug.

Si se agota un recurso y ya existen resultados útiles, Search devuelve
`partial_success`. Sólo se devuelve `failure` si no existe resultado útil y
todas las vías de provider ejecutables han fallado, han sido rechazadas o han
expirado.

## 16. Casos contractuales obligatorios de Fase B

| ID | Escenario | Invariante verificable |
|---|---|---|
| B-03 | Dos tareas solicitan el último slot de provider | Una reserva se confirma; la otra se rechaza con `provider_limit`. |
| B-04 | Provider cancelado antes de iniciar HTTP | Reserva se liquida una vez con consumo cero. |
| B-05 | Provider cancelado tras consulta facturable | Se conserva el coste incurrido; se libera sólo el remanente. |
| B-06 | Retry después de `Retry-After` | No duplica providers ni excede reserva/coste. |
| B-07 | Deadline vence con tareas en curso | No nacen nuevas tareas; cada reserva termina en estado terminal. |
| B-08 | Dos `settle` concurrentes sobre la misma reserva | Una sola liquidación modifica el ledger. |
| B-09 | Lectura HTTP supera byte limit | El siguiente chunk se rechaza y la operación termina con `byte_limit`. |
| B-10 | Browser no disponible | No consume browser call; Deep degrada sin renderer. |
| B-11 | Budget agotado con resultados previos | Estado global `partial_success`. |
| B-12 | Todas las reservas rechazadas o fallidas | Estado global `failure` con error compuesto seguro. |

## 17. Checklist de cierre de Fase B

- [ ] El ledger tiene una única autoridad de mutación dentro del orquestador.
- [ ] Las reservas son atómicas, inmutables y de liquidación idempotente.
- [ ] Ningún provider recibe acceso mutable al Budget global.
- [ ] Cada dimensión de presupuesto tiene punto de reserva y consumo definido.
- [ ] Cancelación, timeout y retry no generan doble consumo ni doble liberación.
- [ ] La concurrencia no mantiene locks durante operaciones externas.
- [ ] Los casos B-03 a B-12 se implementan como contract tests deterministas.
- [ ] Todas las causas de agotamiento usan los nombres canónicos del golden
      template.

---

# AMATL — Fase C: política cuantificada de Search MVP

Documento complementario de diseño. Define valores iniciales conservadores para
decisiones de búsqueda; no reemplaza la calibración por benchmark prevista en
el golden template.

## 18. Objetivo y principios

Esta política convierte los términos operativos de Search en predicados
deterministas. Debe ser versionada como `search_policy = "v1"`, configurable a
nivel de instalación y observable en debug. Los valores de configuración no
alteran contratos ni `schema_version`.

Principios:

- la primera ronda usa dos providers cuando ambos están disponibles; puede usar
  un tercero si sus capacidades satisfacen un filtro explícito no cubierto;
- la expansión sólo intenta mejorar cobertura útil o diversidad real;
- nunca se expande si no queda capacidad presupuestaria o tiempo suficiente;
- el mismo Query, capacidades, estado de providers, telemetría y Budget debe
  producir el mismo plan y la misma decisión de parada;
- los umbrales son valores de producto, no valores mágicos en código.

## 19. Configuración inicial: `search_policy.v1`

```toml
[search_policy]
version = "v1"
first_round_min_providers = 2
first_round_max_providers = 3
minimum_useful_results = 8
target_useful_results = 12
minimum_unique_domains = 4
target_unique_domains = 6
low_diversity_domain_ratio = 0.50
low_diversity_provider_ratio = 0.20
low_diversity_result_type_ratio = 0.20
minimum_marginal_gain = 0.15
minimum_expected_marginal_gain = 0.15
minimum_remaining_deadline_ms = 750
maximum_results_per_domain = 2
maximum_results_per_provider = 5
maximum_results_per_result_type = 6
```

Los campos de configuración están sujetos a validación al inicio:

- mínimos y objetivos son enteros positivos;
- cada objetivo es mayor o igual a su mínimo;
- ratios están en `[0.0, 1.0]`;
- `first_round_min_providers <= first_round_max_providers`;
- `minimum_remaining_deadline_ms` es positivo;
- los máximos de diversity son positivos.

Una configuración inválida no se corrige silenciosamente: se rechaza en el
arranque o se usa el default completo de `v1` con error explícito, conforme a la
política de configuración que se establezca en Fase D/implementación.

## 20. Definiciones computables

### 20.1 Resultado útil

Un `SearchResult` es útil si y sólo si:

1. tiene `original_url` y `canonical_url` válidas;
2. no fue bloqueado por seguridad;
3. está en estado `visible` o `relegated_by_diversity`;
4. posee título no vacío o fallback visual derivable de dominio/path;
5. no está fusionado dentro de otro resultado confirmado.

Un resultado útil no requiere snippet, fecha, provider_rank ni score expuesto.
Los resultados relegados cuentan para cobertura, pero no para el conjunto
visible usado para evaluar diversity de interfaz.

### 20.2 Cobertura

Sea `U` el número de resultados útiles y `D` el número de dominios únicos entre
ellos:

```text
coverage_minimum = U >= minimum_useful_results
                   AND D >= minimum_unique_domains

coverage_target  = U >= target_useful_results
                   AND D >= target_unique_domains
```

`coverage_minimum` habilita la parada por cobertura; `coverage_target` permite
parar sin evaluar providers posteriores salvo que exista un filtro explícito
aún no satisfecho. La comprobación siempre se hace después de normalización,
canonicalización, dedupe, ranking y diversity.

### 20.3 Diversidad baja

Sobre el conjunto visible con `V > 0`:

```text
domain_ratio      = unique_domains / V
provider_ratio    = unique_providers / V
result_type_ratio = unique_result_types / V

low_diversity = domain_ratio < low_diversity_domain_ratio
                OR provider_ratio < low_diversity_provider_ratio
                OR result_type_ratio < low_diversity_result_type_ratio
```

Para `V < 3`, diversity no es evidencia suficiente para expandir por sí sola;
la expansión requiere además falta de `coverage_minimum`. Los ratios se calculan
con valores exactos o precisión decimal consistente; no se redondean antes de
comparar.

### 20.4 Ganancia marginal observada y esperada

Para un provider ejecutado `p`:

```text
marginal_gain(p) = new_unique_useful_results(p) / max(1, provider_queries(p))
```

En MVP, `provider_queries(p)` es uno por ronda. Sólo cuentan resultados útiles
que no existían antes de integrar la salida de `p`.

`expected_marginal_gain(p)` procede de fallback estático en `Bootstrap` o de
telemetría posterior, siempre limitado por las reglas base. Si no hay valor
disponible, se usa el default estático documentado del provider y categoría;
si tampoco existe, se considera `0.0` para decisiones de expansión opcional.

## 21. Decisión de expansión

Un provider candidato puede ejecutarse en una ronda posterior sólo si se cumple
todo lo siguiente:

1. no fue usado ya en la ronda actual ni excluido por estado/configuración;
2. tiene una capacidad relevante para Query o mejora diversidad prevista;
3. hay una reserva Budget concedible;
4. queda al menos `minimum_remaining_deadline_ms` antes del deadline;
5. `coverage_target` aún no se alcanzó;
6. existe uno de estos motivos:
   - no se alcanzó `coverage_minimum`;
   - `low_diversity` es verdadero y hay al menos tres resultados visibles;
   - un filtro explícito aceptado por el candidato no fue cubierto por los
     providers ya usados;
7. `expected_marginal_gain(candidate) >= minimum_expected_marginal_gain`.

Excepción conservadora: si no se alcanza `coverage_minimum` tras primera ronda,
se puede ejecutar el siguiente provider elegible aunque su ganancia esperada no
alcance el umbral, siempre que existan Budget y deadline. Esta excepción ocurre
una sola vez por búsqueda y queda registrada en `debug_reasons`.

## 22. Condiciones de parada

Search deja de programar providers adicionales si se cumple cualquiera de estas
condiciones, evaluadas en este orden:

1. `time_exhausted` o deadline vencido;
2. no queda una reserva factible para ningún candidato;
3. `coverage_target` alcanzado y no hay filtro explícito pendiente;
4. tras alcanzar `coverage_minimum`, todos los candidatos elegibles tienen
   `expected_marginal_gain < minimum_expected_marginal_gain`;
5. un provider posterior produjo ganancia observada menor al mínimo y no queda
   un candidato con capacidad exclusiva solicitada por Query;
6. todos los providers elegibles ya fueron intentados;
7. la política de providers máximos por ronda o global se alcanzó.

La parada no descarta resultados ya obtenidos. Si existe algún resultado útil,
el agotamiento por las condiciones 1, 2 o 7 produce `partial_success` cuando
también haya degradación, provider fallido o cobertura inferior al objetivo.

## 23. Relación con Ranking y Diversity

Los umbrales de esta fase no cambian ranking. Diversity continúa siendo una
etapa posterior que relega, no elimina. Los límites iniciales por dominio,
provider y `result_type` aplican tras ranking y antes de la evaluación visual
de diversidad.

La cobertura cuenta todos los resultados útiles deduplicados, incluidos los
relegados; la diversidad evalúa el conjunto visible. Esta distinción evita que
la relegación artificial convierta una búsqueda bien cubierta en una búsqueda
aparentemente vacía.

## 24. Salida de decisión y trazabilidad

Cada ronda debe producir un registro determinista para debug:

```text
round, providers_considered, providers_selected, providers_skipped,
useful_results, unique_domains, unique_providers, unique_result_types,
coverage_minimum, coverage_target, low_diversity,
expected_marginal_gain_by_provider, stop_reason
```

`stop_reason` usa una causa canónica de Budget cuando corresponda; de otro modo
usa uno de: `coverage_target_reached`, `marginal_gain_low`,
`providers_exhausted`, `explicit_filter_satisfied`.

## 25. Casos contractuales obligatorios de Fase C

| ID | Escenario | Resultado esperado |
|---|---|---|
| C-02 | Ocho resultados útiles y cuatro dominios | Se alcanza `coverage_minimum`. |
| C-03 | Doce resultados útiles y seis dominios | Se alcanza `coverage_target`; no hay expansión opcional. |
| C-04 | Doce resultados de un solo dominio | `low_diversity = true`; se permite candidato diverso si no se agotó Budget. |
| C-05 | Dos resultados visibles | Diversity no inicia expansión por sí sola. |
| C-06 | Cobertura mínima no alcanzada y candidato con ganancia esperada baja | Se permite una única excepción de cobertura. |
| C-07 | Cobertura mínima alcanzada y todos los candidatos bajo umbral | Parada `marginal_gain_low`. |
| C-08 | Filtro explícito no cubierto por primera ronda | Se permite provider capaz aunque haya cobertura objetivo parcial. |
| C-09 | Menos de 750 ms al deadline | No se inicia provider nuevo; causa `deadline_near`. |
| C-10 | Mismos inputs y configuración | Misma selección, expansión y razón de parada. |

## 26. Checklist de cierre de Fase C

- [ ] `search_policy.v1` se valida y versiona independientemente del binario.
- [ ] Resultado útil, cobertura, diversidad y ganancia marginal tienen fórmulas
      explícitas.
- [ ] Expansión y parada producen razones de debug reproducibles.
- [ ] Los umbrales se consumen desde configuración, no desde constantes ocultas.
- [ ] La excepción por cobertura incompleta ocurre como máximo una vez.
- [ ] Ranking y Diversity mantienen sus responsabilidades separadas.
- [ ] Los casos C-02 a C-10 se implementan como contract tests deterministas.

---

# AMATL — Fase D: semántica de Query y contradicciones

Documento complementario de diseño. Esta fase formaliza el parser como única
frontera autorizada para interpretar texto libre, sin alterar `raw_query` ni
permitir reinterpretación por providers.

## 27. Gramática operativa MVP

Los operadores reconocidos no distinguen mayúsculas/minúsculas en su nombre:
`SITE:`, `site:` y `Site:` son equivalentes. Sus valores conservan su forma
original hasta la validación específica.

```text
site:<host>          limita a uno o más hosts
-site:<host>         excluye hosts
filetype:<extension> limita a extensiones sin punto inicial
lang:<tag>           preferencia de idioma BCP 47 simplificada
region:<code>        región ISO 3166-1 alpha-2
before:<YYYY-MM-DD>  límite superior exclusivo
after:<YYYY-MM-DD>   límite inferior exclusivo
exact:<texto>        término/frase literal
"texto"              frase literal
-texto               término excluido
```

Un operador sólo se reconoce cuando empieza en un límite de token y tiene valor
no vacío. Una apariencia de operador incluida dentro de una frase entre comillas
se trata como texto literal. Todo token que no sea un operador válido permanece
en `normalized_query`.

Las fechas se interpretan en UTC:

```text
after:2026-01-01  => 2026-01-01T00:00:00Z exclusivo
before:2026-01-01 => 2026-01-01T00:00:00Z exclusivo
```

Por tanto, una fecha publicada exactamente en el límite no cumple ese filtro.
Los providers que sólo admiten precisión inclusiva reciben el filtro como
`approximated_filter` y no reinterpretan la Query.

## 28. Normalización de valores extraídos

| Operador | Normalización | Validación |
|---|---|---|
| `site`, `-site` | host minúsculo, sin esquema ni path | host DNS/IDN válido; sin wildcard en MVP |
| `filetype` | minúscula, sin punto inicial | 1–16 caracteres alfanuméricos | 
| `lang` | minúscula; subtags separados por `-` | BCP 47 simplificado: 2–3 letras + subtags opcionales |
| `region` | mayúscula | ISO alpha-2 reconocido |
| `before`, `after` | RFC 3339 UTC interno | fecha ISO exacta y calendario válido |
| `exact`, comillas | whitespace interno preservado | texto no vacío y comillas balanceadas |

El parser no hace DNS, no consulta providers y no intenta verificar si un
dominio existe. Un IDN válido se preserva como valor de Query; la conversión a
punycode pertenece a Canonicalization de URL, no a interpretación de intención.

## 29. Precedencia y composición

La precedencia canónica es:

```text
operadores explícitos válidos
  > filtros explícitos normalizados
  > frases exactas (`exact:` y comillas)
  > términos excluidos
  > texto libre para heurísticas léxicas
```

Reglas de composición:

- múltiples `site:` se combinan por unión lógica (OR);
- múltiples `-site:` se combinan por unión lógica;
- múltiples `filetype:` se combinan por unión lógica;
- un término puede aparecer en una frase exacta y en texto libre sólo una vez
  en la representación semántica, preservando siempre `raw_query`;
- `exact:<texto>` y `"texto"` producen elementos separados de `quoted_terms`,
  con deduplicación estable sólo de valores idénticos;
- los términos excluidos no se eliminan del texto bruto, sólo se representan en
  `excluded_terms`.

## 30. Política de contradicciones y valores repetidos

Todos estos casos generan `QueryWarning` con código estable. No causan que los
providers tengan que adivinar el significado.

| Caso | Acción semántica | Warning |
|---|---|---|
| `site:a.com -site:a.com` | Exclusión prevalece; `a.com` se elimina de `domains` y queda en `excluded_domains`. | `domain_included_and_excluded` |
| `site:a.com site:a.com` | Se conserva una sola entrada estable. | Ninguno |
| `-site:a.com -site:a.com` | Se conserva una sola entrada estable. | Ninguno |
| `before` anterior o igual a `after` | Ambos límites de fecha se invalidan y no se aplican. | `invalid_date_range` |
| Fecha inválida | Ese filtro no se aplica; token permanece como literal en `normalized_query`. | `invalid_date_filter` |
| `lang:es lang:en` | Último valor válido prevalece. | `repeated_language_filter` |
| `region:MX region:ES` | Último valor válido prevalece. | `repeated_region_filter` |
| `lang:` o `region:` inválido | Filtro no se aplica; token permanece literal. | `invalid_filter_value` |
| `filetype:pdf filetype:PDF` | Se conserva una sola extensión normalizada. | Ninguno |
| `exact:` vacío o comillas sin cerrar | No se crea frase; token/contenido queda literal. | `invalid_exact_phrase` |
| `site:` con path, esquema o puerto | Filtro no se aplica; token queda literal. | `invalid_domain_filter` |

Esta política elige preservación literal ante valor inválido y exclusión
predecible ante contradicción de dominios. Así se evita perder intención de
usuario de forma silenciosa y se reduce el riesgo de resultados fuera del
alcance solicitado.

## 31. Forma exacta de `QueryWarning`

Los warnings se emiten en orden de aparición del token problemático y nunca
incluyen secretos. La forma serializable se concreta así:

```json
{
  "code": "invalid_date_range",
  "operator": "before",
  "value": "2026-01-01",
  "message": "before must be later than after"
}
```

`code` es estable y apto para automatización. `message` es humano y puede
localizarse en una superficie futura; no debe usarse como contrato. `operator`
y `value` son `null` sólo si el warning no corresponde a un token individual.

## 32. Resultado de parsing y límites entre módulos

El parser devuelve `Query` incluso si existen warnings, excepto cuando
`raw_query` es vacío o contiene únicamente whitespace: en ese caso devuelve un
error de uso tipado, antes de Classification y sin consultar providers.

`normalized_query` contiene:

- texto libre válido;
- tokens de filtros inválidos preservados literalmente;
- no contiene operadores válidos ya extraídos;
- no contiene comillas estructurales de frases válidas.

Los providers consumen campos estructurados de `Query` y pueden declarar filtros
ignorados o aproximados; no vuelven a parsear `raw_query` ni
`normalized_query`. Classification usa los campos de Query conforme a la
precedencia de §29, sin mutarlos.

## 33. Casos contractuales obligatorios de Fase D

| ID | Entrada | Resultado esperado |
|---|---|---|
| D-03 | `rust site:docs.rs -site:docs.rs` | `docs.rs` sólo excluido; warning de contradicción. |
| D-04 | `news after:2026-02-01 before:2026-01-01` | Ambos límites `null`; warning `invalid_date_range`. |
| D-05 | `lang:es lang:en compiladores` | `language = "en"`; warning de filtro repetido. |
| D-06 | `filetype:PDF filetype:pdf tokio` | `file_types = ["pdf"]`; sin warning. |
| D-07 | `site:https://example.com/a` | Token literal; warning `invalid_domain_filter`. |
| D-08 | `exact:"rust async" tokio` | `quoted_terms = ["rust async"]`; texto libre `tokio`. |
| D-09 | `"site:example.com"` | La cadena se trata como frase literal, no como filtro. |
| D-10 | `before:2026-02-30` | Token literal; warning `invalid_date_filter`. |
| D-11 | whitespace únicamente | Error de uso tipado; no hay SearchPlan. |
| D-12 | Misma entrada con distintos case de operador | Query estructurada idéntica. |

## 34. Checklist de cierre de Fase D

- [ ] La gramática reconoce sólo operadores completos y valores válidos.
- [ ] `raw_query` se conserva sin modificación.
- [ ] La precedencia, composición y contradicciones no quedan a criterio de un
      provider.
- [ ] Los warnings tienen códigos estables, orden de aparición y datos seguros.
- [ ] Toda fecha se convierte a UTC con límites exclusivos documentados.
- [ ] Los valores inválidos se preservan como texto literal cuando corresponde.
- [ ] Los casos D-03 a D-12 se implementan como pruebas unitarias y de
      propiedades del parser.

---

# AMATL — Fase F: Ranking MVP reproducible

Documento complementario de diseño. Esta fase especifica la política de ranking
de Search sin introducir embeddings, LLM, authority score opaco ni cambios a
Diversity como etapa posterior.

## 35. Política y alcance

La política se identifica como `ranking_policy = "v1"`. Se aplica a
`DeduplicatedResult` después de canonicalización y dedupe, y antes de Diversity.
Su salida interna asocia a cada candidato un `RankingScore` y sus contribuciones
explicables. `SearchResult` no expone score, RRF ni contribuciones en la salida
normal.

La política usa sólo estas señales, ya definidas en el golden template:

1. RRF de posiciones nativas verificables;
2. coincidencia entre Query y título;
3. coincidencia entre Query y snippet;
4. frescura;
5. acuerdo entre providers.

Las señales ausentes aportan `0.0`; no eliminan el candidato ni cambian las
señales restantes. Todas las puntuaciones intermedias y finales están en
`[0.0, 1.0]` y se conservan en precisión `f64`; sólo se redondean para
presentación de debug.

## 36. Configuración inicial: `ranking_policy.v1`

```toml
[ranking_policy]
version = "v1"
rrf_k = 60
weight_rrf = 0.35
weight_title_match = 0.30
weight_snippet_match = 0.15
weight_freshness = 0.10
weight_provider_agreement = 0.10
freshness_half_life_days = 30
freshness_unknown = 0.0
```

Validación requerida:

- `rrf_k` es entero positivo;
- todos los pesos están en `[0.0, 1.0]`;
- la suma exacta de pesos es `1.0`, con tolerancia técnica máxima de `1e-12`;
- `freshness_half_life_days` es entero positivo;
- toda variación de fórmula, pesos o normalización exige una nueva versión de
  `ranking_policy`, nunca un cambio silencioso de `v1`.

## 37. Preparación determinista de texto

Para coincidencia de Query, título y snippet se emplea la misma preparación
determinista:

1. normalización Unicode NFKC;
2. casefold Unicode;
3. decodificación/limpieza HTML ya realizada en Normalization;
4. colapso de whitespace;
5. tokenización por separadores Unicode no alfanuméricos;
6. eliminación de tokens vacíos;
7. conservación de tokens de una letra cuando provengan de una frase exacta.

El conjunto de términos de Query para ranking contiene `normalized_query` y
`quoted_terms`, sin operadores ni `excluded_terms`. En `v1` no hay stemming,
sinónimos, detección de idioma, embeddings ni expansión semántica. Las frases
exactas se consideran un token compuesto adicional y no sustituyen sus tokens
individuales.

## 38. Señales normalizadas

### 38.1 RRF

Para un candidato `r`, se consideran sólo providers que entregaron
`provider_rank` verificable. Si `rank_p(r)` es la posición nativa de `r` en
provider `p`:

```text
raw_rrf(r) = Σ 1 / (rrf_k + rank_p(r))
rrf(r) = raw_rrf(r) / provider_count_with_rank(r) / (1 / (rrf_k + 1))
```

`rrf(r)` queda normalizado a `[0.0, 1.0]`. Un candidato con rank `1` en todos
los providers que lo reportan obtiene `1.0`; providers sin rank no reducen ni
incrementan la señal. Si no existe ningún `provider_rank`, `rrf(r) = 0.0`.

AMATL no inventa `provider_rank`: conservar orden de llegada, índice local o
posición tras dedupe no satisface el requisito de orden nativo verificable.

### 38.2 Coincidencia con título y snippet

Para un campo textual `T`, se calcula:

```text
token_coverage(T) = matched_distinct_query_tokens(T) / query_tokens_count
phrase_bonus(T)   = matched_exact_phrases(T) / quoted_terms_count
text_match(T)     = min(1.0, 0.85 * token_coverage(T) + 0.15 * phrase_bonus(T))
```

Si la Query no tiene tokens utilizables o el campo está ausente, la señal es
`0.0`. `title_match = text_match(title)` y
`snippet_match = text_match(snippet)`. La señal de snippet participa sólo en el
score combinado; nunca es criterio de desempate.

### 38.3 Frescura

Con una fecha publicada válida, `age_days` es la diferencia no negativa entre
`SearchPlan.ranking_reference_time` y `published_at`, en días decimales:

```text
freshness = 2 ^ (-age_days / freshness_half_life_days)
```

Fechas futuras respecto a `ranking_reference_time` se tratan como `age_days =
0`. Una fecha ausente, inválida o no fiable aporta `freshness_unknown`
(inicialmente `0.0`). La frescura no reemplaza la relevancia textual ni ordena
por fecha de forma independiente. El reloj del sistema no se consulta durante
Ranking: la referencia capturada por el orquestador es el único origen temporal
de `v1`.

### 38.4 Acuerdo entre providers

Sea `P` el número de providers únicos que aportaron el candidato y `A` el total
de providers que devolvieron al menos un resultado normalizado válido durante la
ronda:

```text
provider_agreement = 0.0                         si A <= 1
provider_agreement = (P - 1) / (A - 1)           si A > 1
```

La señal está en `[0.0, 1.0]` y mide corroboración, no volumen. No depende de
providers que fallaron, se saltaron por Budget o no produjeron items válidos.

## 39. Score combinado y orden estable

```text
combined_score =
  weight_rrf                * rrf +
  weight_title_match        * title_match +
  weight_snippet_match      * snippet_match +
  weight_freshness          * freshness +
  weight_provider_agreement * provider_agreement
```

La comparación usa precisión completa de `f64`; dos scores sólo se consideran
iguales cuando su representación numérica es exactamente igual tras el cálculo
determinista. El orden es:

```text
combined_score descendente
→ title_match descendente
→ stable_order ascendente
```

`stable_order` se define al inicio de Ranking como el índice ascendente del
representante deduplicado en la secuencia determinista:

`canonical_url` serializada en UTF-8, seguida de `original_url` y proveedor
representante, todo en orden lexicográfico de bytes.

RRF, `snippet_match`, frescura, acuerdo de providers y Diversity no participan
en desempates. El score tampoco se usa como umbral de descarte: todos los
candidatos válidos siguen hacia Diversity.

## 40. Contrato interno de explicación

```text
RankingExplanation {
  ranking_policy: String,
  rrf: RankingScore,
  title_match: RankingScore,
  snippet_match: RankingScore,
  freshness: RankingScore,
  provider_agreement: RankingScore,
  combined_score: RankingScore,
  tie_break: TieBreakReason
}
```

`TieBreakReason` es `combined_score`, `title_match` o `stable_order`. Este tipo
permanece interno o en modo debug; nunca es requerido por el contrato JSON base.
La explicación debe permitir reconstruir el orden sin consultar telemetría ni
estado mutable externo.

## 41. Casos contractuales obligatorios de Fase F

| ID | Escenario | Resultado esperado |
|---|---|---|
| F-01 | Un provider con `provider_rank = 1` | `rrf = 1.0`. |
| F-02 | Provider sin orden nativo verificable | `provider_rank = null`; RRF no recibe aporte. |
| F-03 | Mismo candidato con rank 1 en dos providers | Acuerdo mayor que cero; RRF normalizado en rango. |
| F-04 | Título ausente, snippet coincidente | `title_match = 0`; score usa snippet y demás señales. |
| F-05 | Fecha ausente | `freshness = freshness_unknown`; candidato se conserva. |
| F-06 | Dos scores idénticos, distinto `title_match` | Gana mayor `title_match`. |
| F-07 | Score y título idénticos | Gana `stable_order` lexicográfico. |
| F-08 | Snippet mayor, score combinado empatado | Snippet no rompe empate. |
| F-09 | Resultado relegado por Diversity | Ranking previo permanece inalterado. |
| F-10 | Mismos inputs, política, `ranking_reference_time` y orden de providers | Scores y orden idénticos. |

## 42. Checklist de cierre de Fase F

- [ ] `ranking_policy.v1` valida pesos, rangos y versión.
- [ ] Cada señal tiene fórmula, rango y tratamiento de ausencia.
- [ ] `provider_rank` sólo procede de orden nativo verificable.
- [ ] RRF es señal combinada, no desempate ni dato público normal.
- [ ] Diversity se ejecuta después de Ranking y no reescribe scores.
- [ ] El orden final se puede reconstruir con `RankingExplanation`.
- [ ] Los casos F-01 a F-10 son contract tests con fixtures golden.

---

# AMATL — Fase E: Canonicalización y deduplicación conservadoras

Documento complementario de diseño. Canonicalización se ejecuta antes de hash,
caché, dedupe y ranking; redirects y `final_url` permanecen fuera de Search.

## 43. Principios e interfaz

```text
canonicalize(original_url) -> CanonicalizationOutcome

CanonicalizationOutcome {
  original_url: OriginalUrl,
  canonical_url: CanonicalUrl,
  transformations: Vec<CanonicalTransformation>,
  status: complete | degraded
}
```

La función es pura, determinista e idempotente. No realiza red, DNS, HTTP ni
resolución de redirects. Si una transformación no es segura, conserva una URL
válida en su forma más conservadora y registra degradación cuando aplique.

```text
canonicalize(canonical_url).canonical_url == canonical_url
```

## 44. Tabla normativa de canonicalización

| Componente | Regla `v1` | Acción / transformación |
|---|---|---|
| Esquema | Sólo `http` y `https`, en minúsculas | Otro esquema: resultado descartado en Normalization. |
| Host | Minúsculas; IDN a punycode | `lowercase_host`, `idn_to_punycode`. |
| Usuario/contraseña | No permitidos en Search | Descartar sin registrar credenciales. |
| Puerto `:80` HTTP / `:443` HTTPS | Se elimina | `remove_default_port`. |
| Otro puerto explícito | Se conserva | Ninguna. |
| Path vacío | Se representa como `/` | `add_root_path`. |
| Path no vacío | Se conserva exactamente | No se agregan/eliminan slash ni segmentos. |
| Percent-encoding | Sólo hexadecimales de escapes a mayúscula | `normalize_percent_hex`; sin decodificar reservados. |
| Query | Conserva orden, claves y valores salvo denylist | Ver §45. |
| Fragmento | Vacío se elimina; no vacío se conserva por defecto | Ver §46. |

Una URL que no se pueda parsear estructuralmente se descarta en Normalization.
La canonicalización no asume equivalencia entre HTTP y HTTPS.

## 45. Parámetros de query

La comparación de nombres de parámetro usa ASCII case-insensitive y preserva
orden relativo de lo que permanece. La única denylist de `v1` es:

```text
utm_*
fbclid
gclid
msclkid
yclid
_ga
_gl
mc_cid
mc_eid
```

`utm_*` requiere el prefijo literal `utm_`. Cada eliminación registra
`remove_tracking_parameter` con el nombre, nunca con el valor. Se conservan
siempre, salvo una regla futura específica y versionada: `ref`, `source`,
`campaign`, `medium`, `id`, `page`, `q`, `query`, `lang`, `locale`, `token`,
`signature` y `expires`.

La conservación de un parámetro no autoriza a registrarlo en logs o telemetría.
Canonicalization nunca elimina parámetros ambiguos para aumentar dedupe.

## 46. Fragmentos y estados

Un fragmento no vacío se conserva en `canonical_url` por defecto, incluidos
anclajes aparentemente simples como `#section-2`: puede identificar contenido
o navegación semántica. Sólo el fragmento vacío se elimina con
`remove_empty_fragment`.

La transformación `remove_nonsemantic_fragment` queda reservada para una
política futura, versionada y respaldada por evidencia específica de que un
fragmento no es semántico para el recurso concreto. No se usa en
`canonicalization_policy.v1`; un patrón sintáctico por sí solo nunca basta.

Transformaciones permitidas:

```text
lowercase_scheme | lowercase_host | idn_to_punycode |
remove_default_port | add_root_path | normalize_percent_hex |
remove_tracking_parameter | remove_empty_fragment |
remove_nonsemantic_fragment
```

Estados: `complete` o `degraded`. Una URL inválida/esquema prohibido es descarte
tipado, no degradación. `degraded` indica que la URL válida sólo recibió cambios
verificablemente seguros.

## 47. Política de deduplicación `dedupe_policy.v1`

El orden obligatorio es:

1. `original_url` idéntica;
2. `canonical_url` idéntica;
3. similitud de título, sólo como `possible_duplicate`;
4. similitud de contenido exclusivamente en Deep.

Los pasos 1 y 2 generan `confirmed_duplicate`. El paso 3 no fusiona ni elimina
resultados. `final_url` no participa en dedupe ni caché de Search.

El representante de una fusión confirmada se elige de forma determinista por:

```text
mayor número de providers distintos
→ presencia de título no vacío
→ presencia de snippet no vacío
→ published_at válida más reciente
→ canonical_url lexicográficamente menor
→ original_url lexicográficamente menor
```

La fusión conserva providers únicos ordenados, ranks por provider, URL
originales únicas, snippets alternativos distintos, fechas observadas y razones
de fusión. No inventa metadata.

## 48. Similitud de título: detección incierta

Para títulos se usa NFKC, casefold Unicode, whitespace normalizado y
tokenización Unicode de Ranking. No se evalúa similitud si falta algún título,
si alguno tiene menos de 4 tokens o menos de 20 caracteres Unicode, o si los
dominios canónicos son iguales.

En los casos elegibles:

```text
title_similarity = intersection(tokens_a, tokens_b) / union(tokens_a, tokens_b)
```

Si `title_similarity >= 0.90`, se marca `possible_duplicate` con razón
`title_similarity_high`; de otro modo, `distinct` para esa señal. No hay
traducciones, stemming, sinónimos ni comparación semántica en Search.

## 49. Dataset de regresión mínimo

| ID | Entrada / pares | Resultado esperado |
|---|---|---|
| E-01 | `HTTP://EXAMPLE.COM:80` | `http://example.com/`; case, puerto y root path normalizados. |
| E-02 | `https://e.com/a?utm_source=x&id=7` | Elimina sólo `utm_source`; conserva `id=7`. |
| E-03 | `https://e.com/a?ref=x&source=y` | Parámetros conservados en el mismo orden. |
| E-04 | `https://e.com/a#section-2` | Conserva el fragmento no vacío. |
| E-05 | `https://e.com/a#/route?tab=1` | Fragmento conservado. |
| E-06 | `https://e.com/a/` vs. `https://e.com/a` | Permanecen distintos en `v1`. |
| E-07 | Misma URL de dos providers | `confirmed_duplicate`, procedencia preservada. |
| E-08 | URLs distintas y títulos largos idénticos | `possible_duplicate`, sin fusión. |
| E-09 | Dos títulos de tres tokens | No se aplica similitud de título. |
| E-10 | Search URL y `final_url` iguales tras redirect | No afecta dedupe de Search. |
| E-11 | URL con credencial embebida | Descartada sin log de credencial. |
| E-12 | Canonicalizar dos veces | Segunda aplicación no cambia URL ni transformaciones. |

## 50. Checklist de cierre de Fase E

- [ ] Canonicalización es pura, determinista e idempotente.
- [ ] Sólo se eliminan parámetros de la denylist explícita.
- [ ] Fragmentos y slash final se tratan conservadoramente.
- [ ] No hay DNS, redirects ni equivalencia HTTP/HTTPS en Search.
- [ ] Dedupe confirmado exige URL original o canónica idéntica.
- [ ] Similitud de título sólo produce `possible_duplicate`.
- [ ] La fusión conserva procedencia y representante estable.
- [ ] E-01 a E-12 son fixtures de contrato y regresión.

---

# AMATL — Fase G: gobernanza y activación de providers

Documento complementario de diseño. Ningún adapter puede activarse sólo por
existir: requiere ficha vigente de términos, coste, límites y aprobación. La
información externa fue revisada el **2026-08-12** y debe revalidarse cada 90
días, antes de un release o ante cualquier cambio de términos/adapter.

## 51. Puerta común de activación

```text
ProviderApproval {
  provider, adapter_version, reviewed_at, reviewer,
  terms_url, terms_version_or_date, allowed_access_method,
  plan_or_contract, rate_limit, cost_model, storage_rights,
  supported_regions, supported_filters, data_handling_notes,
  operational_risk, approval_status
}
```

`approval_status` es `draft`, `approved`, `expired` o `rejected`. Sólo
`approved` permite `enabled`; cualquier ficha incompleta o vencida deshabilita
el provider. Todo adapter debe usar secretos sólo desde entorno, respetar cuota
y `Retry-After`, identificarse honestamente y mantener cache/retención apagadas
salvo derechos contractuales explícitos.

## 52. Brave Search API (`stable`)

| Campo | Decisión |
|---|---|
| Método | API oficial autenticada; prohibido scraping de resultados web. |
| Credencial | `X-Subscription-Token` desde variable de entorno. |
| Capacidades verificadas | Web search, `country`, `search_lang`, conteo y offset. |
| Coste/capacidad observada | USD 5/1,000 requests y 50 QPS publicados; confirmar al contratar. |
| Términos | Search API Terms of Use, actualizados 2026-02-11. |
| Riesgo clave | Los términos restringen almacenar/cachear resultados salvo almacenamiento transitorio o derechos explícitos. |

`ProviderSearchCache` de Brave queda **deshabilitada por defecto**. Sólo puede
activarse con evidencia de derechos de almacenamiento para el plan contratado,
TTL y borrado definidos. La política de privacidad publicada indica retención
de consultas hasta 90 días para facturación/troubleshooting: debe informarse en
la evaluación de privacidad de cualquier superficie expuesta.

Fuentes oficiales: [API y precios](https://brave.com/search/api/),
[términos](https://api-dashboard.search.brave.com/documentation/resources/terms-of-service),
[restricciones](https://api-dashboard.search.brave.com/app/documentation/general/terms-of-service)
y [privacidad](https://api-dashboard.search.brave.com/privacy-policy).

## 53. Mojeek Search API (`stable`)

| Campo | Decisión |
|---|---|
| Método | API oficial JSON/XML; no scraping de interfaz de búsqueda. |
| Capacidades verificadas | Site search, country/language boosting, clustering, SafeSearch y límites de resultados, sujetos a plan. |
| Coste/capacidad observada | GBP 2/3 CPM para planes Startup/Business, 5/10 QPS publicados; confirmar cotización y contrato. |
| Retención | La página comercial declara storage rights; el entitlement efectivo debe constar en contrato. |
| Riesgo clave | Capacidades, precio y cache dependen del plan; el uso de contenido tercero exige respetar derechos de sus editores. |

`ProviderCapabilities` se construye desde una matriz versionada por plan, no
desde supuestos del parser. Cache sólo se activa cuando `storage_rights` sea
afirmativo y se documente la retención aplicable.

Fuentes oficiales: [documentación](https://www.mojeek.com/support/api/),
[Search API](https://www.mojeek.com/support/api/search/) y
[planes/capacidades](https://www.mojeek.com/services/search/web-search-api/).

## 54. DuckDuckGo HTML (`best_effort`)

| Campo | Decisión |
|---|---|
| Método | HTML no autenticado, experimental; no es API oficial aprobada. |
| Coste/cuota | No hay plan API oficial verificado en esta revisión. |
| Autorización de scraping | No se encontró permiso oficial específico en las fuentes revisadas. |
| Riesgo | Fragilidad, bloqueos y cumplimiento por automatizar una interfaz de usuario. |
| Estado inicial | `disabled_pending_explicit_approval`. |

No puede habilitarse sin autorización verificable para ese método, revisión
legal aplicable y límites documentados. Aun aprobado, su fallo aislado nunca
causa fallo global si otro provider entrega resultados. La
[política de privacidad](https://duckduckgo.com/privacy) no constituye permiso
de automatización y no se interpreta como tal.

## 55. Matriz operativa de Fase 1

| Provider | Default runtime | Cache | Acción ante fallo |
|---|---|---|---|
| Brave | Habilitable sólo con aprobación y token | Apagada sin entitlement | Continuar. |
| Mojeek | Habilitable sólo con aprobación/credenciales | Sólo con derecho contractual | Continuar. |
| DuckDuckGo HTML | Deshabilitado hasta aprobación explícita | Apagada | Continuar; `best_effort`. |

Un provider sin aprobación, credenciales o cuota se omite con degradación
explícita; no se hace fallback no autorizado.

## 56. Evidencia y pruebas obligatorias

Antes de merge/activación, cada ficha debe registrar endpoint y método
autorizados, versión documental, fixture sanitizada, mapeo de filtros,
comportamiento 401/403/429/5xx, límites reales, coste por llamada, derechos de
cache y datos transmitidos. Un cambio de endpoint, query o parseo reinicia la
revisión.

| ID | Escenario | Resultado esperado |
|---|---|---|
| G-01 | Brave sin storage rights aprobado | Cache deshabilitada. |
| G-02 | Brave sin token | Provider omitido, sin red ni secreto en logs. |
| G-03 | Mojeek sin capacidad contratada | Filtro `ignored`/`approximated`, no simulado. |
| G-04 | Ficha vencida | Provider fuera de `selected_providers`. |
| G-05 | DuckDuckGo sin aprobación | Adapter deshabilitado. |
| G-06 | DuckDuckGo aprobado falla | Search continúa con otros providers. |
| G-07 | 429 con `Retry-After` | Retry sujeto a Budget. |
| G-08 | Cambio de términos/endpoint | Ficha vuelve a `draft`. |

## 57. Checklist de cierre de Fase G

- [ ] Cada provider tiene ficha vigente, fuente oficial y responsable.
- [ ] ToS, cuota, coste, acceso y cache se aprueban por provider.
- [ ] Brave no cachea resultados sin entitlement explícito.
- [ ] Mojeek refleja capacidades y contrato reales.
- [ ] DuckDuckGo HTML no se habilita sin permiso verificable.
- [ ] G-01 a G-08 existen como pruebas de configuración/contrato.

---

# AMATL — Fase H: Deep aislado y persistencia tolerante a fallos

Documento complementario de diseño. Deep es posterior al MVP y nunca invalida
Search. Chromium, Trafilatura, SQLite y cache son capacidades opcionales o
auxiliares: su ausencia, fallo o desactivación no afecta correctness de Search.

## 58. Principios operativos

- Sólo `DeepOrchestrator` activa Fetcher, Renderer y Extractor, con Budget
  restante y URL aprobada.
- Chromium no forma parte del binario ni de la instalación base; se detecta en
  runtime y se omite si no está disponible.
- SQLite/cache son optimizaciones descartables: un fallo equivale a cache miss
  o telemetría no persistida.
- El cuerpo completo pertenece a `Document` y a cache documental explícita,
  nunca a `SearchResult`, logs normales ni telemetría.
- Retención/caché respetan la gobernanza de providers de Fase G.

## 59. Contrato operativo del Renderer Chromium

Chromium es un proceso externo opcional, gestionado mediante CDP. Por cada
llamada crea un perfil temporal de uso único, sin extensiones, sync,
credenciales, cookies persistentes ni perfil de usuario.

| Control | Regla |
|---|---|
| Activación | Sólo Deep, Fase 5+, URL aprobada y browser budget reservado. |
| Sandbox | Obligatorio si la plataforma lo soporta; no hay fallback silencioso a modo inseguro. |
| Red | URL aprobada y redirects revalidados; sin loopback, red privada/link-local ni infraestructura interna. |
| Navegación | Una página principal; popups, descargas y navegación no solicitada bloqueados. |
| Recursos | Timeout, CPU, memoria, bytes y redirects sujetos a Budget. |
| Limpieza | Terminar proceso y borrar perfil aun en timeout/cancelación. |

Si el sandbox o un límite requerido no es aplicable de forma verificable, el
renderer se declara no disponible. Deep continúa sin render y devuelve una
degradación tipada: `renderer_unavailable`, `renderer_timeout`,
`renderer_blocked` o `renderer_failed`.

```text
URL aprobada → reservar browser call → crear perfil temporal → iniciar proceso
→ navegar y revalidar redirects → capturar DOM permitido → terminar proceso
→ borrar perfil → liquidar reserva
```

La llamada consume browser budget al iniciar Chromium/navegación. Si no inicia,
libera la reserva. Al vencer deadline se intenta cierre breve y luego
terminación forzada; nunca se conserva sesión entre documentos o búsquedas.

Configuración inicial calibrable:

```toml
[deep.renderer]
enabled = false
max_browser_calls = 2
timeout_ms = 8000
shutdown_grace_ms = 500
max_memory_mb = 512
max_redirects = 5
```

`enabled = false` es obligatorio por defecto. Todo límite declarado requiere
enforcement real; de no poder aplicarse, la ejecución se rechaza.

## 60. SQLite y concurrencia

SQLite usa `WAL`, `busy_timeout = 5000` y `synchronous = NORMAL`. El pool es
pequeño y benchmark-calibrado. Las transacciones abarcan sólo cambios locales:
nunca red, parsing, render o extracción.

1. Escrituras no críticas son best-effort y con timeout limitado.
2. Lectura fallida, bloqueo o base ausente equivalen a cache miss.
3. Error de escritura no bloquea la entrega de SearchResult.
4. Migraciones son independientes de `schema_version`.
5. Corrupción deja la base fuera de uso y la rota a cuarentena fechada; no se
   sobrescribe ni borra automáticamente. `amatl doctor` lo reporta.

## 61. ProviderSearchCache y cache documental

La clave de `ProviderSearchCache` es:

```text
provider | adapter_version | normalized_query | structured_filters
```

Los filtros se serializan de forma canónica/ordenada. La cache conserva sólo
semántica de `ProviderResult` y no guarda headers, tokens o debug sensible. Se
mantiene deshabilitada globalmente hasta que se habilite explícitamente y la
ficha de provider de Fase G otorgue derechos de retención.

```toml
[cache.provider_search]
enabled = false
ttl_seconds = 300
max_entries = 10000
max_bytes = 268435456
eviction = "lru"
```

Invalidación: cambio de adapter, TTL, LRU, límite de tamaño o acción manual.
La cache documental sólo se habilita explícitamente para Deep y usa:

```text
canonical_url | content_hash | extractor_version
```

Guarda tamaño, tipo de contenido, fecha de obtención, método de fetch, estado y
versión. El cuerpo completo sólo se conserva si política local y derechos de
fuente/provider lo permiten. No se indexa, añade a logs ni transmite a terceros.

## 62. Retención, borrado y observabilidad

| Artefacto | Default | Al exceder límite |
|---|---|---|
| ProviderSearchCache | Deshabilitada | LRU/TTL sólo si fue aprobada. |
| Document cache | Deshabilitada | LRU/TTL explícitos por instalación. |
| Telemetría persistida | Opcional | Poda de métricas agregadas. |
| SQLite corrupta | Cuarentena | Revisión o purga explícita. |
| Perfil Chromium | Sólo durante llamada | Limpieza obligatoria. |

Purgas, borrados y cuarentena requieren acción explícita en interfaz
administrativa/CLI y no interrumpen Search activo. Los eventos permitidos son
`component`, `status`, `elapsed_ms`, `size_bytes`, `budget_cause`, `cache_hit`,
`cache_eviction`, `renderer_available`, `extractor_used` y `degradation_code`.
No se registran por defecto cuerpos, cookies, tokens, auth headers, contraseñas
ni query strings sensibles.

## 63. Casos contractuales obligatorios de Fase H

| ID | Escenario | Resultado esperado |
|---|---|---|
| H-01 | Chromium ausente | Deep continúa sin render; Search intacto. |
| H-02 | Sandbox/límite no exigible | Renderer no disponible; no hay modo inseguro. |
| H-03 | Redirect a IP privada durante render | Bloqueo y liquidación de reserva. |
| H-04 | Deadline vence durante render | Proceso y perfil se limpian; Search persiste. |
| H-05 | SQLite bloqueada/corrupta | Cache miss/degradación; Search continúa. |
| H-06 | Falla telemetría persistida | Métricas en memoria continúan. |
| H-07 | Brave sin storage rights | Cache de provider no lee/escribe. |
| H-08 | Cambia adapter/extractor version | Cache anterior no se reutiliza. |
| H-09 | Cache documental alcanza cuota | Evicción LRU; no afecta documento actual. |
| H-10 | Purga solicitada | Acción explícita; Search activo no se afecta. |

## 64. Checklist de cierre de Fase H

- [ ] Deep no se ejecuta desde Search ni sin Budget restante.
- [ ] Chromium es opcional, aislado y sin fallback inseguro.
- [ ] Perfil, proceso y navegación se limpian ante todo cierre.
- [ ] SQLite/cache no participan en correctness de Search.
- [ ] Las claves incluyen versiones de adapter/extractor cuando aplica.
- [ ] Retención y borrado respetan Fase G y acción explícita.
- [ ] H-01 a H-10 son pruebas de seguridad, integración y contrato.

---

# AMATL — Cierre de contratos faltantes

Complemento aprobado para cerrar las fronteras pendientes sin modificar el
golden template. Los defaults numéricos son configurables y benchmark-calibrated.

## 65. Normalization y errores de Search

Normalization distingue valores `reported` por provider de valores `derived` por
AMATL mediante metadata de procedencia. Limpia entidades HTML, encoding y
whitespace; no inventa metadata. URL inválida/esquema no permitido descarta sólo
el item. Título ausente conserva item con fallback visual; snippet corrupto se
elimina; fecha inválida se convierte en `null` y conserva el original en
metadata; encoding irrecuperable elimina sólo el campo; metadata parcial
conserva entradas válidas; respuesta inesperada produce `invalid_response`.

Se añade a `SearchResponse`, de modo aditivo, `errors: Vec<CompositeError>`.
Siempre se serializa como `[]` en `success`. `CompositeError` contiene
`code`, `message`, `providers` y `recoverable`; no contiene secretos, headers o
contenido. `failure` exige al menos un error y ningún resultado útil;
`partial_success` conserva resultados y errores/degradaciones pertinentes.

## 66. Provider y Parallel Search

`Provider::search` recibe `Query`, filtros estructurados, deadline, reserva de
Budget y límites; devuelve `ProviderResult`. No puede mutar Query, SearchPlan,
ranking, dedupe, Diversity ni Budget global. Parallel Search controla deadline
global, timeout individual, concurrencia global/per-provider, cancelación y
resultados parciales. Un timeout individual nunca excede tiempo restante.

Sólo se reintentan `timeout`, `network`, `rate_limit`, `unavailable` y 5xx
transitorios, con `Retry-After`, backoff exponencial y jitter; máximo dos
reintentos y siempre sujeto a la misma reserva/Budget. Nunca se reintentan auth,
parser determinista, configuración inválida o bloqueo explícito. Salida:
`provider_results`, `providers_used`, `providers_failed`, `providers_partial`,
`elapsed`, `budget_remaining`.

## 67. Provider Value y Diversity

`ProviderValueSnapshot` contiene provider, categoría, estado, ventana,
muestra, `unique_results`, duplicate ratio, top-K contribution, success rate,
timeout rate, latency, diversity y coste. Bootstrap tiene menos de 100 consultas
válidas por provider/categoría; Learning de 100 a 499; Mature desde 500. Ventana
de 30 días con decay exponencial; la telemetría nunca altera límites de
seguridad. Cada provider elegible mantiene exploración mínima de 10%.

`diversity_policy = v1`: máximo 2 visibles por dominio, 5 por provider y 6 por
`result_type`. Se recorren candidatos ya rankeados y los que exceden un límite
pasan a `relegated_by_diversity`, sin eliminarse. Un candidato puede superar el
límite si su `combined_score` es al menos 15% mayor que el siguiente candidato
visible del mismo grupo limitado (dominio, provider o `result_type`) en el orden
previo a Diversity. Si no existe candidato visible de ese grupo, no hay override.
La decisión registra límite, grupo, score, comparador y override; no reescribe
Ranking ni desempates.

## 68. Stubs Deep y fronteras Fetcher/Extractor

`Document` conserva obligatoriamente `search_result_id`, original/canonical/
final URL, content_hash, fetch_method, extractor_used, content_type, size,
retrieved_at, status y schema_version. `Evidence`, `Gap` y `SubQuery` son stubs
serializables con `schema_version`; no participan en Search MVP. `Gap` contiene
`gap_type`, `severity`, `reason`, `recommended_query`, `estimated_cost` y
`expected_gain`. Sólo DeepOrchestrator ejecuta una SubQuery, máximo 0–2 y con
Budget.

Fetcher recibe URL aprobada, timeout, byte/redirect limits y headers permitidos;
devuelve `final_url`, `status`, `headers_safe`, `content_type`, `body`, `size`,
`redirect_chain` y `retrieved_at`. `final_url` es transitorio en `FetchResult` y
sólo `Document` lo persiste o expone como dato de Deep. Permite sólo HTTP/HTTPS,
valida SSRF antes de conectar, tras DNS cuando aplique y tras cada redirect; no
ejecuta contenido. Extractor no hace networking ni render, y devuelve `content`,
`format`, `title`, `author`, `published_at`, `metadata`, `extractor_used` y
`status`. Si Trafilatura no existe/falla, Deep conserva Document superficial y
reporta degradación.

## 69. Fixtures de cierre

| ID | Caso | Resultado |
|---|---|---|
| X-01 | Snippet corrupto / título ausente | Sólo snippet se elimina; item se conserva con fallback. |
| X-02 | Todos providers fallan | `failure` y `errors` no vacío. |
| X-03 | 429 con Retry-After | Retry limitado por Budget; no duplica consumo. |
| X-04 | Provider con 99/100/500 muestras | Bootstrap/Learning/Mature respectivamente. |
| X-05 | Tercer resultado mismo dominio | Relegado salvo override de 15%. |
| X-06 | Redirect a red privada | Fetch bloqueado; SearchResult permanece. |
| X-07 | Trafilatura ausente | Document superficial y degradación. |
| X-08 | Gap propone tercera variante | Rechazada por límite de dos SubQueries. |

## 70. Checklist final

- [ ] Normalization cubre campo corrupto, procedencia y respuesta inesperada.
- [ ] `errors` es aditivo, seguro y consistente con SearchStatus.
- [ ] Provider/Parallel Search cubren timeout, retry, cancelación y parcialidad.
- [ ] Provider Value y Diversity tienen política versionada y defaults aprobados.
- [ ] Document, Evidence, Gap y SubQuery tienen fronteras sin entrar al MVP.
- [ ] Fetcher/Extractor aplican seguridad y degradación especificadas.
- [ ] X-01 a X-08 son contract tests de merge.

---

# Cierre controlado de hallazgos

## 71. Gobernanza de este documento

Este documento se usa sólo como backlog técnico y matriz de trazabilidad. Las
secciones que proponen defaults, fórmulas o umbrales llevan estado
`proposal_pending`; las reglas repetidas literalmente desde `plan_amatl.md`
llevan estado `derived`. Antes de implementar una regla `proposal_pending`, su
decisión debe promoverse al golden template mediante el proceso que el proyecto
autorice. El golden prevalece siempre.

## 72. Catálogo cerrado de tipos auxiliares

| Tipo | Forma canónica | Dueño | Expuesto/persistido |
|---|---|---|---|
| `Degradation` | `code`, `component`, `message`, `context_safe` | frontera que degrada | Sí, con `schema_version` si cruza frontera. |
| `CompositeError` | `code`, `message`, `providers`, `recoverable` | execution | Sí, dentro de `SearchResponse`. |
| `MergeReason` | enum `original_url_exact`, `canonical_url_exact` | Deduplication | Sí. |
| `CanonicalTransformation` | enum de transformaciones permitidas | Canonicalization | Sí. |
| `CostEstimate` | `amount_units`, `unit`, `currency` opcional | Provider | Sí. |
| `BudgetExhaustionCause` | enum canónico de §7.7 | Budget | Sí. |
| `ProviderExecutionStatus` | `success`, `partial`, `failure` | Provider | Sí. |
| `BudgetRemaining` | límites y capacidad restante por dimensión | Budget | Debug/interno. |
| `Settlement` | `reservation_id`, `consumed`, `released`, `state`, `cause` | Budget | Debug/interno. |
| `Evidence` | `schema_version`, `document_id`, `status` | Deep stub | Sí, post-MVP. |
| `SubQuery` | `schema_version`, `raw_query`, `reason`, `status` | Deep stub | Sí, post-MVP. |

Todos los enums se serializan en `snake_case`; `context_safe` y mensajes nunca
contienen secretos, headers, cuerpos ni credenciales. `SearchResponse` queda
con una sola forma canónica: los campos de §4.8 más `errors: Vec<CompositeError>`;
en éxito se serializa `errors: []`.

## 73. Provider Value y Router completos

`ProviderValueSnapshot` existe en dos vistas: `global` por provider y
`by_category` por provider/categoría. Ambas contienen ventana, muestra, éxito,
timeout, latencia, coste, resultados únicos, duplicación, contribución top-K y
diversidad. La ventana es de 30 días y el peso de una observación de edad `d`
días es `2^(-d/30)`. La vista por categoría se usa cuando tiene muestra mínima;
en otro caso se combina con global mediante promedio ponderado por muestra. Los
estados Bootstrap/Learning/Mature y exploración mínima mantienen los umbrales
propuestos marcados `proposal_pending`.

`Router::recommend` recibe Query, Classification, ProviderCapabilities,
ProviderValueSnapshot, health y límites solicitables. Devuelve sólo providers
ordenados, `provider_budget_requests`, fallback, exclusiones y razones debug.
En Bootstrap usa fallback estático; telemetría sólo ajusta prioridades y nunca
seguridad, reglas base ni Budget. El orquestador convierte solicitudes en
`provider_budgets`; Router no puede crear ni modificar reservas.

## 74. Matriz única de contract tests

Cada celda debe existir antes de merge; los IDs nuevos se reservan para fixtures
que aún no están materializados.

| Módulo | Válido | Degradado | Error | Parcial | Budget | Invariantes |
|---|---|---|---|---|---|---|
| Provider | P-01 | P-02 | P-03 | P-01 | P-04 | P-05 |
| Router | R-01 | R-02 | R-03 | R-04 | R-05 | R-06 |
| Budget | B-03 | B-04 | B-08 | B-11 | B-01 | B-03..B-12 |
| Normalization | N-03 | N-02 | N-04 | N-05 | N-06 | N-01..N-06 |
| Canonicalization | CA-01 | CA-03 | CA-04 | CA-05 | CA-06 | E-01..E-12 |
| Deduplication | D-01 | D-02 | D-03 | D-04 | D-05 | D-01..D-05 |
| Ranking | F-01 | F-04 | F-05 | F-09 | F-10 | F-01..F-10 |
| Diversity | DV-01 | DV-02 | DV-03 | DV-04 | DV-05 | DV-06 |
| Fetcher | X-06 | X-07 | FE-01 | FE-02 | FE-03 | FE-04 |
| Extractor | EX-01 | X-07 | EX-02 | EX-03 | EX-04 | EX-05 |

Los IDs aún no definidos son deuda explícita de fixture, no evidencia de prueba
existente. Cada fixture debe validar entrada, salida tipada, `schema_version`
cuando aplique, ausencia de secretos y causa de Budget cuando corresponda.

## 75. Registro final de coherencia

| Hallazgo | Estado | Criterio de cierre |
|---|---|---|
| Fuente normativa única | Cerrado | §71 declara el complemento no normativo. |
| Tipos indefinidos / dos SearchResponse | Cerrado | §72 consolida tipos y respuesta. |
| Provider Value global/categoría | Cerrado como propuesta | §73 define ambas vistas y combinación. |
| Router sin contrato | Cerrado | §73 fija entrada, salida y no-ownership de Budget. |
| Cobertura de tests dispersa | Cerrado como matriz | §74 es requisito de merge; los fixtures reservados siguen pendientes de implementación. |

Resultado de auditoría: no hay contradicción técnica activa con el golden
template mientras este documento permanezca no normativo. Las reglas marcadas
`proposal_pending` requieren promoción explícita antes de implementarse.
