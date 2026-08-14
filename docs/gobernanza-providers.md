# Gobernanza de providers

Que un adapter compile no autoriza su uso. Cada provider cruza una puerta
individual, renovable y fail-closed; `providers.enabled` sólo selecciona
candidatos ya aprobados.

## Puerta de activación

Una ficha es aprobada sólo cuando `approval_status = "approved"`, `reviewed_at`
es una fecha ISO válida con antigüedad máxima de 90 días y existen valores no
vacíos para adapter version, reviewer, terms URL y versión/fecha, método de
acceso, plan/contrato, rate limit, coste, notas de datos y riesgo operativo
(`config.rs`). Además, el adapter real exige habilitación y credencial
cuando corresponda. Una ficha expirada o incompleta produce provider no
disponible; no existe fallback que omita la puerta.

La puerta es ejecutable en tiempo de ejecución, no sólo declarativa: en cada
búsqueda el servicio omite construir una fuente habilitada cuya ficha no esté
aprobada y añade la degradación `provider_not_approved` con su nombre
(`service.rs`). Una ficha que vence mientras el proceso está vivo deja de
enviar tráfico en la siguiente búsqueda, sin reinicio y sin edición del
archivo. `POST /reload` y `SIGHUP` aplican una ficha renovada de inmediato.

Independientemente de la gobernanza, un cortacircuitos persistente retira una
fuente que acumula fallos consecutivos durante su ventana de enfriamiento
(`provider_circuit_open`) y permite después una sonda; ver
`docs/operacion.md`.

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
| DuckDuckGo HTML | `best_effort` | **Sin implementación.** `duckduckgo.rs` es un stub de 73 líneas: sin endpoint, sin cliente HTTP y sin parseo; `search()` devuelve error siempre | ficha vacía `draft` | Siempre no disponible: `provider_pending_explicit_approval` | Implementación completa, autorización verificable y ficha. No es un adapter apagado: no existe |

La fecha `2026-02-11` existente en el default de Brave es
`terms_version_or_date`; **no** es `reviewed_at`. No se registra como revisión.
Mojeek tiene URL de soporte pero no una versión/fecha de términos. Ningún revisor
está definido. Por ello, el estado correcto actual es bloqueado para tráfico
real aunque existan adapters (`config.rs:318-350`, `service.rs:277-353`,
`providers/duckduckgo.rs:8-59`).

## Viabilidad y coste de las fuentes

Verificado el **2026-08-14**. Esta sección describe el mercado, no el código:
cambia sin que el repositorio cambie, y debe revisarse junto con cada ficha.

La puerta de gobernanza no exige pagar nada. `cost_model` es un campo de texto:
`"0"` o `"free tier"` son valores válidos y aprueban igual. Lo que exige es que
el coste esté **declarado**, sea cual sea.

| Fuente | Coste real | Clave | API de búsqueda web | Cobertura |
|---|---|---|---|---|
| Brave Search API | **De pago desde 2026-02.** Tarjeta obligatoria al registrarse; 5 USD/mes de crédito (~1 000 consultas) y luego cobro por uso | Sí | Sí, oficial | Índice propio amplio |
| Mojeek Search API | De pago | Sí | Sí, oficial | Índice propio, medio |
| Marginalia | **0** para uso no comercial (CC-BY-NC-SA) | Sí, gratuita por correo; clave `public` para pruebas | Sí, oficial (`api2.marginalia-search.com`) | Índice propio pequeño, orientado a web independiente |
| SearXNG autohospedado | **0**, sin cuota | No | Sí, la de tu instancia (`format=json`, desactivado por defecto) | Agrega Google, Bing, DuckDuckGo y otros |
| DuckDuckGo | 0 | No | **No existe.** Sólo Instant Answer API, que devuelve definiciones y resúmenes, no resultados web | — |

Consecuencia práctica: **gratuito, amplio y con términos permisivos no coexisten
en una sola fuente.** Toda elección sacrifica una de las tres.

### Sobre los términos y qué significa el riesgo

El riesgo de usar una fuente cuyos términos prohíben la consulta automatizada no
es judicial en la práctica: es **operativo**. El origen empieza a devolver
captchas o bloquea la dirección IP, y la fuente deja de servir resultados. Eso lo
absorbe el cortacircuitos como cualquier otro fallo, pero degrada el buscador de
forma permanente en lugar de transitoria.

Por eso `operational_risk` debe recoger esa exposición explícitamente y no
tratarse como un trámite.

### SearXNG como fuente

Es la única opción gratuita con cobertura amplia, y traslada un problema en lugar
de eliminarlo: no tiene índice propio, sino que consulta motores cuyos términos
pueden prohibir el acceso automatizado. La ficha debe declararlo:

- `terms_url` y `terms_version_or_date` apuntan a **tu** instancia, que no tiene
  términos propios; se trata de una autocertificación y conviene decirlo en
  `data_handling_notes` en lugar de dejar el campo aparentando una revisión
  externa que no existe.
- `allowed_access_method = "self_hosted"`, `cost_model = "0"`,
  `plan_or_contract = "self-hosted"`.
- `operational_risk` debe nombrar la dependencia de los motores upstream y la
  posibilidad de bloqueo por reputación de IP a volumen alto.

Configurar la instancia con sólo motores de términos permisivos reduce ese riesgo
a costa de cobertura. Es una decisión del operador, no del core.

Nota de implementación: `ProviderRuntimeConfig` no tiene campo para la URL de la
instancia ni mapa libre, de modo que una factory de SearXNG debe resolver su
dirección por variable de entorno o requiere ampliar la configuración.

### Alcance de red de los providers

El guard anti-SSRF (`validate_resolved_addresses`) protege **Deep**, no el
transporte de providers, que usa `ReqwestTransport` sin validación de dirección.
Una fuente autohospedada en `127.0.0.1` o en la red local es por tanto
alcanzable, lo que hace viable SearXNG. La contrapartida es que una ficha
aprobada puede apuntar a infraestructura interna: es coherente con el modelo —los
providers los configura el operador— pero debe ser una decisión consciente.

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
   El adapter se expone mediante un `ProviderFactory` registrado en
   `ProviderRegistry` con el mismo nombre que la ficha; declarar la ficha sin
   registrar la factory produce `provider_not_registered` y no tráfico real.
6. Ejecutar el gate completo. Sólo entonces añadirlo a `providers.enabled`.
7. Renovar antes de 90 días o marcar `expired`/`rejected`. Un cambio de ToS,
   método, precio o contrato exige revisión inmediata, no esperar la caducidad.

`storage_rights = false` impide que la caché escriba resultados de ese provider.
No debe activarse por conveniencia técnica: requiere un derecho verificado.

## Fuentes de terceros

`[providers]` es un mapa abierto y `AmatlService::with_registry` acepta un
registro propio, de modo que un integrador puede añadir fuentes sin modificar el
núcleo. La puerta de gobernanza no cambia: la ficha completa y vigente sigue
siendo obligatoria, `provider-canary` la verifica antes de cualquier acceso de
red, y una factory puede declarar `supports_network_canary = false` o
`requires_credential = false` cuando su fuente no admite canary real o no usa
credencial.
