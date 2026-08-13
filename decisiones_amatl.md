# Decisiones de arquitectura — AMATL

| ID | Decisión | Consecuencia |
|---|---|---|
| D-001 | `plan_amatl.md` y `fase_a_contratos.md` son rectores e inmutables en desarrollo ordinario. | Los cambios se implementan y documentan fuera de ambos archivos. |
| D-002 | Las políticas versionadas validan forma, rangos y pesos, pero sus umbrales son calibrables. | Ajustar valores válidos no exige recompilar contratos ni cambiar `schema_version`. |
| D-003 | Ranking v2 se habilita sólo si supera Ranking MVP sobre el corpus humano etiquetado y versionado. | Un fixture sintético autocumplido no constituye evidencia de aceptación. |
| D-004 | La UI usa bearer token introducido manualmente y no crea sesión/cookie. | Evita estado de autenticación implícito; el operador debe disponer de `AMATL_SERVER_TOKEN`. |
| D-005 | DuckDuckGo HTML permanece bloqueado hasta autorización verificable. | Su adapter nunca participa por existir en el binario. |
| D-006 | `contract-gate` es el gate de pull request para formato, tests, Clippy, Audit, Deny y SBOM. | La protección de rama debe marcarlo como requerido en el hosting Git. |
| D-007 | `scraper` 0.27 reemplaza el parser HTML manual para descubrimiento estructural de enlaces. | Se admite explícitamente MPL-2.0 para sus dependencias transitivas; `fxhash` quedó eliminado del grafo. |
