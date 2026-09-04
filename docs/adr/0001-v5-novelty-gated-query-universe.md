# ADR-0001: Universo de queries gobernado por novedad y diversidad

- Estado: aceptado
- Fecha: 2026-09-04
- Contexto: AMATL, análisis post-V5 (`independent-relevance-confirmatory-v5`)

## Decisión

AMATL adoptará un único control arquitectónico para futuros confirmatorios: un
**query universe novelty-gated**. El universo no podrá congelarse hasta que un
preflight offline/controlled haya medido novedad contra el corpus histórico,
diversidad efectiva entre familias semánticas y concentración esperada de
resultados. La admisión será por estratos de familia semántica y dominio (si el
proveedor los expone), y conservará balance 1:1 entre Treatment y Control.

Esto es una sola arquitectura de admisión del universo; no autoriza V6 ni una
nueva ejecución en este work package.

## Evidencia V4/V5

V5 está cerrado como `COMPLETE_INCONCLUSIVE` porque agotó su universo congelado.
El proveedor respondió 2172/2172 requests HTTP exitosamente; por tanto, no hay
evidencia de indisponibilidad de SearXNG.

| Métrica | V4 | V5 |
|---|---:|---:|
| Queries / brazo | 300 | 1086 |
| Raw results | 1000 | 3870 |
| Valid results | 312 | 627 |
| Treatment / Control válidos | 205 / 107 | 402 / 225 |
| Duplicados actuales | 414 | 1354 |
| Solapamiento histórico | 274 | 1889 |

Reconstrucción V5:

```text
FROZEN_QUERIES       = 2172
EXECUTED_QUERIES     = 2172
RAW_RESULTS          = 3870
CANONICAL_RESULTS    = 3870 records / 916 unique canonical URLs
DUPLICATE_CURRENT    = 1354
HISTORICAL_OVERLAP   = 911 + 167 + 811 = 1889
FINAL_ACCEPTED       = 627
```

La reconciliación es exacta: `1354 + 1889 + 627 = 3870`. Los indicadores son:

```text
RAW_RESULTS_PER_QUERY  = 1.782
VALID_RESULTS_PER_QUERY = 0.289
VALID_RATE_RAW          = 16.20%
REJECTION_RATE_RAW      = 83.80%
DUPLICATE_CURRENT_RATE  = 34.99%
HISTORICAL_OVERLAP_RATE = 48.81% de registros raw
```

Los motivos representan del raw: `DUPLICATE_CURRENT 34.99%`, `OVERLAP_V1
23.54%`, `OVERLAP_V3 4.32%`, `OVERLAP_V4 20.96%` y aceptados `16.20%`.
No hubo `INVALID_RESULT`, `OTHER_REJECTIONS` ni fallos HTTP.

## Saturación y duplicación

Aplicando las mismas reglas de agrupación del ejecutor V5 a los artefactos
históricos disponibles: `UNIQUE_URLS_V1=555`, `V2=10`, `V3=82`, `V4=382`;
`UNION_HISTORICAL_URLS=923`. V5 recuperó `916` URLs canónicas únicas, de las
cuales `289` ya estaban en la unión histórica y `627` fueron nuevas.
Así, `V5_HISTORICAL_OVERLAP_RATE=31.55%` por URL única y `48.81%` por registro
raw. Las intersecciones históricas V1/V2, V1/V3, V1/V4, V2/V3, V2/V4 y V3/V4
son respectivamente `9, 23, 62, 1, 2 y 18` URLs.

La clasificación es `CORPUS_SATURATION=SEVERE`: casi la mitad de cada registro
raw queda contaminado por histórico, sólo 627 URLs nuevas sobreviven, y el
resto del universo después de deduplicar (`2516`) se divide exactamente entre
1889 históricos y 627 aceptados.

Los 1354 duplicados son: `SAME_QUERY=0`, `CROSS_QUERY=1015`,
`CROSS_PAIR=34`, `CROSS_ARM=305`, `OTHER=0`. La deduplicación se concentra en
322 URLs; las diez primeras explican 22.16% de los duplicados. Las principales
son `vixra.org/astro` (46), `millermicro.com/` (44),
`unabomber.neocities.org/` (34), `greenmagi.com/` (34) y la página de Vatican
(31). Los diez dominios principales concentran 24.30% de los duplicados.

## Query universe y brazos

Las 2172 queries son textualmente únicas (`QUERY_UNIQUENESS=100%`), pero su
diversidad efectiva es `LOW`: se generan como 1086 topics emparejados, sólo 31
familias de términos por 36 contextos, y el Treatment usa únicamente tres
operadores (`what is`, `how does`, `what causes`); el Control es una paráfrasis
estructural del mismo topic. El resultado es nominalmente grande, pero
estructuralmente sintético. Hay 604 dominios raw y 916 URLs canónicas únicas;
76.33% de los registros raw son repeticiones de una URL ya vista. Se clasifica
`RESULT_SET_OVERLAP=HIGH/CONCENTRATED`, aunque el Jaccard medio entre todos los
conjuntos no vacíos es sólo 0.0055 por la presencia de muchos conjuntos
disjuntos o vacíos; hubo 541 pares de conjuntos no vacíos idénticos.

Por brazo, V5 tuvo Treatment `2342 raw`, `402 valid`, duplicados `802`, V1
`610`, V3 `92`, V4 `436`; Control tuvo `1528 raw`, `225 valid`, duplicados
`552`, V1 `301`, V3 `75`, V4 `375`. Los yields raw son `17.16%` y `14.73%`.
La diferencia 402/225 está explicada parcialmente por volumen raw desigual y
distinta mezcla de contaminación/duplicación, no demuestra mayor relevancia:
`ARM_YIELD_ASYMMETRY=PARTIALLY_EXPLAINED`.

## Canonicalización

La función V5 baja scheme/host, elimina fragmentos, quita sólo el slash final
del path y preserva query parameters. En el ledger hubo 16 URLs con query y 8
con fragment; no hubo grupos donde varias URLs originales colapsaran en una
misma canónica. El cambio observado fue principalmente la normalización del
slash final (746 registros). No hay false collapse demostrado:
`CANONICALIZATION_FALSE_COLLAPSE_RISK=LOW`.

## Causa raíz e hipótesis

| Hipótesis | Estado | Evidencia / impacto |
|---|---|---|
| H1: baja diversidad efectiva | SUPPORTED | 31 familias × 36 contextos y operadores limitados; produce queries nominalmente distintas pero repetitivas. |
| H2: concentración SearXNG | SUPPORTED | 916 URLs únicas frente a 3870 raw; duplicados concentrados en URLs/dominios concretos. |
| H3: saturación histórica | SUPPORTED | 1889 registros rechazados por V1/V3/V4; 289/916 URLs únicas históricas. |
| H4: duplicación interna | SUPPORTED | 1354 registros, 34.99%; principalmente cross-query. |
| H5: sobre-colapso canónico | NOT SUPPORTED | query preservada, path preservado y cero grupos de múltiples originales. |
| H6: limitación de proveedor único | INSUFFICIENT_EVIDENCE | V5 no compara providers; sí demuestra el límite del espacio visible a SearXNG. |
| H7: comportamiento específico de brazo | PARTIALLY_SUPPORTED | raw y composición difieren, pero no implica relevancia y no explica por sí solo el déficit. |

`PRIMARY_ROOT_CAUSE=H1_QUERY_UNIVERSE_LOW_EFFECTIVE_DIVERSITY`.
`SECONDARY_ROOT_CAUSES=H2_SEARXNG_RESULT_CONCENTRATION,
H3_HISTORICAL_CORPUS_SATURATION,H4_INTERNAL_DUPLICATION`.

La capa dominante es `B. QUERY STRATEGY LIMIT`; la capa `A. SEARCH ENGINE
LIMIT` contribuye mediante concentración de resultados y la capa `C. EVALUATION
CORPUS LIMIT` amplifica el déficit por contaminación histórica. V5 no prueba que
el producto sea incapaz de encontrar resultados útiles fuera de este corpus.

## Consecuencias

El preflight tendrá que calcular antes de congelar: novedad estimada, diversidad
efectiva, contaminación histórica, yield conservador, balance por brazo y
capacidad para `474/474`. Gates iniciales, ajustables sólo con nueva evidencia
documentada: cota inferior conservadora de `>=474` válidos por brazo; balance
esperado `1:1` con ratio de yields no peor que `1.25`; `QUERY_UNIQUENESS=100%`;
solapamiento textual/estructural dentro de una familia controlado y una
proyección de contaminación histórica que deje margen para 474 por brazo.
El gate debe usar validación post-dedup y post-histórico, no raw-result yield.

El alcance de implementación futura es el generador/admisor del universo y su
preflight auditable. No se cambia V1–V5, ground truths, canonicalización,
resultados históricos, proveedor registrado en V5, ni se ejecuta V6.

`PRODUCT_IMPACT=BOTH`: el hallazgo es directamente experimental por saturación
del corpus, y también afecta la estrategia real de adquisición porque el
generador actual sobreproduce consultas con baja novedad efectiva.

## Preparación para reconsiderar un confirmatorio

`FUTURE_CONFIRMATORY_READINESS=NOT_READY` hasta que el nuevo admisor produzca
todos los gates cuantitativos anteriores con evidencia congelada. La siguiente
acción única derivada de la causa raíz es:

`NEXT_SINGLE_ACTION=Implementar el preflight de admisión novelty-gated del query universe (diversidad efectiva, novedad histórica y capacidad conservadora por brazo).`

## Integridad del work package

No se ejecutaron requests de red, V6, Marginalia ni cambios de código
productivo. V5 conserva sus artefactos y resultados.

