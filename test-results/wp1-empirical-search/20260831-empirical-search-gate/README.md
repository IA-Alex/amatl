# WP-1 — Empirical Search Gate (2026-08-31)

## Alcance y reproducibilidad

- Providers reales autorizados: `marginalia` y `searxng`.
- Cohorte congelada: 10 consultas heterogéneas, una ejecución secuencial por
  consulta, con 3 s de intervalo entre posiciones.
- Dataset: `test-results/benchmarks/searxng-v2/20260823-190535/dataset.json`
  (`sha256: 92ce0225bfde9f55d7a4abce156d9fc4f4bdab885c1eedf30bfa8b1cc6899b3a`).
- Fixture de ejecución: `wp1-authorized-providers.toml`
  (`sha256: c7012a3d1128386cbf33d4781f94a3a5bde8b64a59e74651be05f5546e972a6f`).
  Desactiva persistencia, cachés, reintentos y cortacircuito para que el estado
  previo no altere la cohorte.
- Binario reconstruido: `target/release/amatl` `0.1.0-rc.1`
  (`sha256: 20033d9361b6dbd59d904689e41ade78e69ba36e7dfa3ad5df8e6831692b8d63`).

## WP-2 aplicado antes de la repetición

La primera consulta conjunta reveló un defecto interno: `success` podía
acompañar a `results: []` cuando un provider respondía vacío y otro fallaba.
WP-2 redujo `no_usable_results` a la condición contractual correcta: el
conjunto final vacío nunca es éxito. La prueba de regresión
`empty_successful_provider_does_not_mask_a_peer_failure` cubre exactamente
ese caso. `cargo fmt --all -- --check`, `cargo test --workspace --locked` y
`cargo clippy --workspace --all-targets -- -D warnings` terminaron con código
0 tras la corrección.

## Resultado de la repetición WP-1

| Posición | Estado | Latencia (ms) | Marginalia | SearXNG | Resultados finales |
|---|---:|---:|---|---|---:|
| Q01 | failure | 1272 | `provider_rate_limit` | ejecutado; 0 utilizables | 0 |
| Q02 | failure | 833 | `provider_rate_limit` | ejecutado; 0 utilizables | 0 |
| Q03 | failure | 820 | `provider_rate_limit` | ejecutado; 0 utilizables | 0 |
| Q04 | failure | 2908 | `provider_rate_limit` | ejecutado; 0 utilizables | 0 |
| Q05 | failure | 836 | `provider_rate_limit` | ejecutado; 0 utilizables | 0 |
| Q06 | failure | 1366 | `provider_rate_limit` | ejecutado; 0 utilizables | 0 |
| Q07 | failure | 822 | `provider_rate_limit` | ejecutado; 0 utilizables | 0 |
| Q08 | failure | 820 | `provider_rate_limit` | ejecutado; 0 utilizables | 0 |
| Q09 | failure | 809 | `provider_rate_limit` | ejecutado; 0 utilizables | 0 |
| Q10 | failure | 804 | `provider_rate_limit` | ejecutado; 0 utilizables | 0 |

Clasificación: `success=0`, `partial_success=0`, `failure=10`. Latencia
agregada: media `1129 ms`, p50 `827.5 ms`, p95 (nearest-rank) `2908 ms`, rango
`804–2908 ms`.

El aislamiento de fallos sí se observó: los diez resultados compuestos marcan
`marginalia` como fallido **y** `searxng` como usado; por tanto el rate limit
de Marginalia no canceló la llamada a SearXNG. El estado agregado es
correctamente `failure` porque ninguno produjo un resultado utilizable.

No hubo candidatos finales ni candidatos compartidos. Por ello las métricas de
normalización/canonicalización y deduplicación no son observables en esta
cohorte (`0` resultados normalizados, `0` URLs canónicas, `0` deduplicaciones
confirmadas); no se las declara PASS.

## Causa raíz y decisión

- Marginalia devolvió `provider_rate_limit` en las diez posiciones compuestas
  y en el control aislado de Q02: bloqueo de cuota del provider externo.
- SearXNG fue consultado correctamente por AMATL, pero no devolvió resultados
  utilizables. La consulta directa controlada a la misma instancia para Q02
  devolvió `results=0`, `answers=0`, `suggestions=0` y
  `unresponsive_engines=[]`; por tanto esta ausencia ocurre antes de la
  normalización/deduplicación de AMATL. La evidencia disponible no identifica
  un motor upstream concreto, así que no se atribuye una causa más específica.

```
WP1_EMPIRICAL_SEARCH_GATE=BLOCKED_EXTERNAL
WP2_INTERNAL_DEFECT=FIXED_AND_REGRESSION_TESTED
WP3_FINAL=COMPLETED_WITH_EXTERNAL_BLOCK
```

No hay base empírica para declarar AMATL cerrado. La continuación requiere que
el operador restaure disponibilidad real de al menos una fuente autorizada y
vuelva a ejecutar esta misma cohorte; no se requiere otro cambio interno por
este bloqueo.
