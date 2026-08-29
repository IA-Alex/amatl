Generated benchmark artifact — AMATL SearXNG Baseline v2 post-change — do not treat as project documentation.

# Hallazgos

## ABORT BENCHMARK

- El dataset exacto de v1 fue leído y contiene Q01–Q10.
- El aislamiento exclusivamente SearXNG está disponible mediante el fixture existente; Marginalia no fue invocado.
- La configuración actual de SearXNG no fue accesible para confirmar `duckduckgo = disabled`, `mojeek = disabled` y `qwant = disabled`.
- Tampoco se observó un listener TCP local en el puerto 8888; no se emitió una solicitud de prueba porque sería una ejecución adicional no permitida.
- La evidencia histórica del cambio de configuración fue observada pero no se usó como sustituto del estado vivo.

No hubo ejecuciones, resultados, errores públicos, cambios de motor, cambios de configuración ni métricas agregadas. La clasificación de comparación es `NOT_COMPARABLE`.
