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
| Brave Search API | `stable` | `brave-v1`; API oficial; credencial `BRAVE_API_KEY` | **`rejected`**, no habilitado | **Descartado por política del operador** (fuente de pago; no es papeleo pendiente) | Ninguno — no se completa la ficha salvo cambio explícito de política |
| SearXNG (autohospedado) | `stable` | `searxng-v1`; API JSON; URL via `SEARXNG_INSTANCE_URL` (por defecto `http://127.0.0.1:8888`); sin credencial | `draft`, no habilitado | Aprobable: adapter completo; sólo faltan `reviewer`/`reviewed_at`/`approval_status` | Ficha completa en `config.rs`; ver «Fichas de aprobación» |
| Marginalia Search API | `stable` | `marginalia-v1`; API oficial `api2.marginalia-search.com`, header `API-Key`, credencial `MARGINALIA_API_KEY` | `draft`, no habilitado | Aprobable: adapter completo y probado; sólo faltan `reviewer`/`reviewed_at`/`approval_status` | Ficha completa en «Fichas de aprobación» |
| Mojeek Search API | `stable` | `mojeek-v1`; API oficial; credencial `MOJEEK_API_KEY` | **`rejected`**, no habilitado | **Descartado por política del operador** (fuente de pago; no es papeleo pendiente) | Ninguno — no se completa la ficha salvo cambio explícito de política |

**Brave y Mojeek están descartados, no pendientes.** `builtin_provider_records()`
(`config.rs`) fija `approval_status = "rejected"` para ambos por defecto, con el
motivo explícito en `cost_model`/`operational_risk` ("Rejected by operator
policy: no paid search providers"). Un test dedicado
(`paid_providers_are_rejected_by_default_not_merely_draft`) falla si esto
regresa silenciosamente a `draft`. Reactivarlos exige una decisión explícita de
política, no simplemente completar `reviewer`/`reviewed_at` — a diferencia de
SearXNG y Marginalia, que sí son papeleo pendiente sobre una decisión ya
tomada (usarlas).

`duckduckgo_html` se retiró del registro y de `providers/`: era un stub de 73
líneas sin endpoint, cliente HTTP ni parseo, y DuckDuckGo no ofrece API de
búsqueda web (sólo Instant Answer, que no devuelve resultados web). Mantenerlo
listado como provider era engañoso; ver «Viabilidad y coste» más abajo.

La fecha `2026-02-11` existente en el default de Brave es
`terms_version_or_date`; **no** es `reviewed_at`. No se registra como revisión.
Mojeek tiene URL de soporte pero no una versión/fecha de términos. Ningún
revisor está definido para ninguno de los dos, pero eso ya no es lo que los
bloquea: `approval_status = "rejected"` los bloquea por decisión de política,
con o sin revisor. Ambos quedan bloqueados para tráfico real aunque existan
adapters (`config.rs`, `service.rs:277-353`).

## Fichas de aprobación: SearXNG y Marginalia

Esta sección fija las fichas de gobernanza para las dos fuentes gratuitas
verificadas. La puerta de activación (`config.rs:603-626`, `approved_on`) exige,
además de `approval_status = "approved"` y un `reviewed_at` vigente (≤ 90 días),
valores no vacíos para `adapter_version`, `reviewer`, `terms_url`,
`terms_version_or_date`, `allowed_access_method`, `plan_or_contract`,
`rate_limit`, `cost_model`, `data_handling_notes` y `operational_risk`. Una
ficha que omita `plan_or_contract` o `rate_limit` **no aprueba**, aunque el
resto esté completo. `adapter_version` debe coincidir con el valor registrado en
`ProviderRegistry` (`searxng-v1`, `marginalia-v1`), no un literal genérico.

### SearXNG (autohospedado) — aprobable

El adapter está implementado y probado (`providers/searxng.rs`); la ficha
incorporada en `config.rs` ya declara método, contrato, cuota, coste, derechos,
notas de datos y riesgo. Sólo faltan `reviewer`, `reviewed_at` y
`approval_status`, que son específicos del operador:

```toml
[providers.searxng]
adapter_version = "searxng-v1"
approval_status = "approved"
reviewed_at = "2026-08-14"         # fecha real de la revisión, ≤ 90 días
reviewer = "identidad verificable" # identidad real, no un rol
terms_url = "https://docs.searxng.org/"
terms_version_or_date = "self-certified"
allowed_access_method = "self_hosted"
plan_or_contract = "self-hosted"
rate_limit = "unlimited (self-hosted)"
cost_model = "0"
credential_env = "SEARXNG_INSTANCE_URL"
storage_rights = false
supported_regions = []
supported_filters = []
data_handling_notes = "Instancia SearXNG autohospedada; sin términos externos. Los motores upstream tienen sus propios términos — el operador debe verificar cumplimiento. AMATL no almacena datos."
operational_risk = "Depende de motores de búsqueda upstream que pueden bloquear el acceso automatizado. Riesgo de reputación de IP a volumen. El operador debe configurar sólo motores permisivos."
```

`terms_url` apunta a la documentación de la instancia, no a una revisión externa
que no existe: es una autocertificación y así se declara en
`terms_version_or_date = "self-certified"` y en `data_handling_notes`.

### Marginalia — aprobable

El adapter está implementado y probado (`providers/marginalia.rs`): consulta
`api2.marginalia-search.com/search` (el endpoint `api.marginalia.nu` original
está deprecado), envía la clave en el header `API-Key` y traduce `site:` de
forma nativa. Sólo faltan `reviewer`, `reviewed_at` y `approval_status`,
específicos del operador:

```toml
[providers.marginalia]
adapter_version = "marginalia-v1"
approval_status = "approved"
reviewed_at = "YYYY-MM-DD"          # fecha real de la revisión
reviewer = "identidad verificable"
terms_url = "https://www.marginalia.nu/" # verificar URL de términos
terms_version_or_date = "..."        # versión/fecha real de los términos
allowed_access_method = "official_api"
plan_or_contract = "..."             # plan/contrato verificado
rate_limit = "..."                   # cuota real, no "TBD"
cost_model = "0"                     # uso no comercial (CC-BY-NC-SA)
credential_env = "MARGINALIA_API_KEY"
storage_rights = false
supported_regions = []
supported_filters = []
data_handling_notes = "Uso no comercial (CC-BY-NC-SA). Índice pequeño orientado a web independiente."
operational_risk = "Dependencia de la API externa (api2.marginalia-search.com); cuota y posible bloqueo por automatización."
```

Notas sobre la ficha de Marginalia:

- `operational_risk` **no** es "nulo": depende de una API externa con cuota y
  posible bloqueo. Declararlo como "sin dependencias externas" sería falso.
- `terms_url` y `terms_version_or_date` deben ser verificados, no hipotéticos.
- `adapter_version` debe ser `marginalia-v1` (el valor registrado en
  `ProviderRegistry`), no `v1`.

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
