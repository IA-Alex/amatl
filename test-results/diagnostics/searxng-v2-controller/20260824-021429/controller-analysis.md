# Análisis del controlador

## Flujo reconstruido

`main()` carga dataset → `build_plan()` genera el producto cartesiano round-major (Q01..Q10 por repetición) → `validated_plan()` ejecuta validación de conteo, unicidad, cobertura y orden → `execute_plan()` crea `ExecutionState` y recorre `for position in plan.plan.positions` → `_execute_position()` valida pertenencia, límite y duplicado, llama al executor, incrementa contador y agrega `ExecutionRecord`.

La ruta `AmatlProcessExecutor.__call__()` valida límite (30), proveedor/query, construye el comando y usa `subprocess.run(..., check=False, shell=False, timeout=50)`. Errores de inicio/timeout se convierten en un resultado `EXECUTOR_FAILURE`; JSON inválido también se convierte en resultado, no se propaga. `execute_plan()` no inspecciona `FAILURE` ni `exit_code` para cortar.

## Q09-R1 → Q10-R1

El plan local contiene Q09-R1 seguido de Q10-R1; ambas posiciones tienen la misma estructura (`benchmark_id`, proveedor `searxng`, repetición 1) y sólo cambia `query_id`/texto de consulta. No hay condición dependiente de Q09, Q10, longitud, caracteres, metadata o serialización que detenga el bucle. La única frontera explícita relevante es el límite `>=30`, que no se alcanza en la transición 9→10.

## Terminación

No existe `break`, `sys.exit`, `return` prematuro en el bucle, ni rama por `FAILURE`/`process_exit_code != 0`. Las excepciones de executor observables se absorben en `AmatlProcessExecutor`; las excepciones de validación sólo ocurren antes del bucle o por violación de barreras. No hay stdout/stderr de una terminación tras Q09 en los artefactos inventariados. Por tanto, la primera etapa demostrable de terminación y la causa del corte son **UNKNOWN**; cualquier señal externa, excepción del proceso envolvente o artefacto de otra versión no puede probarse aquí.

Evidencia adicional: el `execution-plan.json` y `runs.jsonl` existentes describen 30 posiciones/30 registros, contradiciendo el estado conocido de 9/0. Se conserva como discrepancia, no como prueba de que aquellas ejecuciones correspondan a la campaña reportada.

**CONTROLLER_TERMINATION_CAUSE: UNKNOWN**
