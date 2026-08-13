# Gobernanza de providers

Que un adapter compile no autoriza su uso. Cada provider cruza una puerta
individual, renovable y fail-closed; `providers.enabled` sólo selecciona
candidatos ya aprobados.

## Puerta de activación

Una ficha es aprobada sólo cuando `approval_status = "approved"`, `reviewed_at`
es una fecha ISO válida con antigüedad máxima de 90 días y existen valores no
vacíos para adapter version, reviewer, terms URL y versión/fecha, método de
acceso, plan/contrato, rate limit, coste, notas de datos y riesgo operativo
(`config.rs:218-247`). Además, el adapter real exige habilitación y credencial
cuando corresponda. Una ficha expirada o incompleta produce provider no
disponible; no existe fallback que omita la puerta.

Ficha obligatoria:

```toml
[providers.nombre]
adapter_version = "..."
approval_status = "approved" # draft | approved | expired | rejected
reviewed_at = "YYYY-MM-DD"
reviewer = "identidad verificable"
terms_url = "https://..."
terms_version_or_date = "..."
allowed_access_method = "official_api"
plan_or_contract = "..."
rate_limit = "..."
cost_model = "..."
credential_env = "NOMBRE_VARIABLE"
storage_rights = false
supported_regions = []
supported_filters = []
data_handling_notes = "..."
operational_risk = "..."
```

No se aceptan valores hipotéticos para reviewer, fecha, coste, cuota o derechos.
La persona que revisa debe conservar evidencia de términos y contrato fuera del
repositorio si contienen información privada.

## Estado actual verificable

| Provider | Nivel de diseño | Adapter | Config default | Estado efectivo | Datos pendientes |
|---|---|---|---|---|---|
| Brave Search API | `stable` | `brave-v1`; API oficial; credencial `BRAVE_API_KEY` | `draft`, no habilitado | No disponible | reviewer/reviewed_at vigentes, plan, cuota, coste, derechos, datos y riesgo |
| Mojeek Search API | `stable` | `mojeek-v1`; API oficial; credencial `MOJEEK_API_KEY` | `draft`, no habilitado | No disponible | versión/fecha ToS y todos los campos de revisión comercial/operativa |
| DuckDuckGo HTML | `best_effort` | adapter bloqueado sin acceso de red | ficha vacía `draft` | Siempre no disponible: `provider_pending_explicit_approval` | autorización verificable, ToS/coste y ficha completa antes de implementar acceso |

La fecha `2026-02-11` existente en el default de Brave es
`terms_version_or_date`; **no** es `reviewed_at`. No se registra como revisión.
Mojeek tiene URL de soporte pero no una versión/fecha de términos. Ningún revisor
está definido. Por ello, el estado correcto actual es bloqueado para tráfico
real aunque existan adapters (`config.rs:318-350`, `service.rs:277-353`,
`providers/duckduckgo.rs:8-59`).

## Alta o renovación

1. Abrir un provider request usando la plantilla, sin código de acceso de red.
2. Verificar fuente oficial, método permitido, región, filtros, autenticación,
   cuota, coste, almacenamiento y tratamiento de datos.
3. Registrar riesgos: estabilidad de contrato, rate limit, disponibilidad,
   cambios de HTML, bloqueo, privacidad y dependencia operativa.
4. Obtener revisión humana identificable y fecharla; configurar caducidad
   implícita de 90 días.
5. Implementar adapter con errores tipados, límites, sanitización y fixtures sin
   secretos; añadir contract tests de filtros, fallos, cuota y respuesta inválida.
6. Ejecutar el gate completo. Sólo entonces añadirlo a `providers.enabled`.
7. Renovar antes de 90 días o marcar `expired`/`rejected`. Un cambio de ToS,
   método, precio o contrato exige revisión inmediata, no esperar la caducidad.

`storage_rights = false` impide que la caché escriba resultados de ese provider.
No debe activarse por conveniencia técnica: requiere un derecho verificado.
