# Análisis del controlador

Generated static controller/persistence diagnostic — no AMATL or provider execution performed.

## Preservación y contradicción de evidencia

El SHA-256 del runner coincide con el valor solicitado: `28e4b64c68533b65c55ab180a063254387f7b99d7985887a2a899f3152ae02c7`. No se observó `RUNNER_DRIFT_OBSERVED`.

La ruta histórica inspeccionada no muestra el incidente descrito: `runs.jsonl` tiene 30 líneas, con secuencia 1..30, incluidas Q10-R1 (secuencia 10) y Q10-R3 (secuencia 30). `metrics.json` declara `completed_runs: 30`, `recorded_attempts: 30`, `unique_positions: 30`, `missing: 0` y `status: "complete"`. El literal de estado inválido no existe en los ficheros locales buscados. Por ello, los números 9 reportadas/0 durable no son una propiedad demostrable de la campaña actualmente disponible en esa ruta.

## Flujo exacto del runner inspeccionado

1. `build_plan()` crea 30 `Position` en orden round-major; `validated_plan()` llama `validate_plan()` antes de ejecutar.
2. `execute_plan()` (líneas 178--187) crea un `ExecutionState` vacío e itera todas las posiciones con `for position in plan.plan.positions`.
3. `_execute_position()` verifica posición miembro, límite (`attempt_count >= planned_count` o `>=30`) y duplicado; calcula `sequence_number` como `attempt_count + 1`; llama al executor; sólo al retorno añade la posición, incrementa el contador y añade el record en memoria.
4. `AmatlProcessExecutor.__call__()` valida límite propio y query/proveedor, compone el vector `[binary, --config-file, fixture, search, query, --json]` y usa `subprocess.run(..., check=False, capture_output=True, text=True, timeout=50)`. Timeout/OSError se convierten en resultado `EXECUTOR_FAILURE`; JSON inválido también se convierte en resultado, no se propaga.
5. Pero `main()` no conecta `execute_plan()` con ese executor para una campaña. El modo `--amatl-process-integration` fija `Position(..., "Q01", 1)`, construye executor con `max_invocations=1` y llama `execute_integration_position()` (una posición). El modo normal es offline; el modo mock usa sólo `LocalMockExecutor`.

Por tanto, el SHA inspeccionado no contiene una frontera que pueda realizar nueve ni treinta invocaciones AMATL en una campaña. Un wrapper/harness no localizado, una versión distinta usada en ejecución, o artefactos sustituidos serían necesarios para explicar las afirmaciones operativas; ninguno se puede elegir con esta evidencia.

## Q09-R1 a Q10-R1

Ambas comparten `benchmark_id`, `provider="searxng"` y `repetition=1`; cambian `query_id` (Q09→Q10), texto de consulta, y secuencia derivada (9→10). Q09 es `eventual consistency distributed systems`; Q10 es `history of public libraries`. El vector de argumentos tiene la misma forma y no hay condición por longitud, caracteres, query-id, o secuencia 10. La evidencia real además registra Q10-R1, exit code 1 y timestamp `2026-08-24T02:07:36.608870+00:00`, tres segundos después de Q09-R1.

## Condiciones de parada y excepciones

En `execute_plan()` no hay `break`, retorno temprano ni condición sobre `FAILURE` o `process_exit_code != 0`. El límite sólo bloquea una invocación cuando se intenta exceder 30; no puede bloquear secuencia 10. `PlanAbort` de prevalidación, posición ajena/duplicada o executor se propaga porque `execute_plan()` no lo captura; una excepción inesperada de executor también. El executor propio absorbe OSError/timeout y fallos de parseo en un resultado; por sí solos no causan terminación.

No hay stdout, stderr, traceback, log, fichero temporal, contador parcial ni señal externa conservados para el supuesto corte. No existe excepción observable ni primera etapa de terminación demostrable. La primera frontera que *podría* propagar una terminación, si hubiera ocurrido, es `result = executor(position)` en `_execute_position()`; no hay evidencia de que ocurriera entre Q09 y Q10.

## Clasificaciones

- Para la ruta observable hay 30 registros artefactuales **CONFIRMED_EXECUTED** (no prueban por sí solos un subprocess). Respecto de la afirmación externa «9 reportadas», la clasificación correcta es **UNKNOWN**: no existe su transcript ni contador fuente.
- `CONTROLLER_TERMINATION_CAUSE: UNKNOWN`. No es ROOT_CAUSE_CONFIRMED ni LIKELY_CAUSE: el evento alegado contradice los artefactos y no tiene señal local atribuible.

## Validación futura propuesta, sin ejecutar

TEST A: mock que interrumpe después de completar posición 9. Esperado: planned=30; mock invocations=9; durable=9; unique durable=9; duplicates=0; retries=0; secuencia=1..9; posición 10 ausente; estado `INVALID_INTERRUPTED`.

TEST B: mock completo. Esperado: planned=30; invocations=30; durable=30; unique=30; duplicates=0; missing=0; retries=0; secuencia=1..30.

TEST C: tras 30, intento 31 rechazado antes de executor y durable permanece 30. Incluir posición ajena bloqueada antes de executor.
