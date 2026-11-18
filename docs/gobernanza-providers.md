# Gobernanza de providers

Que un adapter compile no autoriza su uso. Cada provider cruza una puerta
individual, renovable y fail-closed; `providers.enabled` sólo selecciona
candidatos ya aprobados.

## Puerta de activación

Una ficha es aprobada sólo cuando `approval_status = "approved"`, `reviewed_at`
es una fecha ISO válida con antigüedad máxima de 90 días y existen valores no
vacíos para adapter version, reviewer, terms URL y versión/fecha, método de
acceso, plan/contrato, rate limit, coste, notas de datos y riesgo operativo
(`config/mod.rs:114-137`). Además, el adapter real exige habilitación y credencial
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
| Marginalia Search API | `stable` | `marginalia-v1`; API oficial `api2.marginalia-search.com`, header `API-Key`, credencial `MARGINALIA_API_KEY` | `draft`, no habilitado | Aprobable: adapter completo y probado; sólo faltan `reviewer`/`reviewed_at`/`approval_status` | Ficha completa en `config/providers.rs`; ver «Fichas de aprobación» |
| Fuentes retiradas (pago y sin API web) | **retiradas del producto** | adapters y fichas eliminados de `providers/` | no declaradas, no registradas | **Retiradas por política del operador** (fuentes de pago y fuentes sin API de búsqueda web; no es papeleo pendiente) | Ninguno — esos nombres se rechazan en `Config::validate()` salvo cambio explícito de política |

**Marginalia es la única fuente que se sirve.** AMATL publica exactamente una
fuente: `marginalia`, que exige `MARGINALIA_API_KEY`. El registro
(`ProviderRegistry::builtin()`) despacha sólo `marginalia-v1`.

**Las fuentes de pago se retiraron del producto.** La política
del operador ejecuta sólo fuentes sin coste, así que no se declaran en los
expedientes incorporados, no están
registradas en `ProviderRegistry` y sus módulos de adapter se eliminaron de
`providers/`. Además, al desaparecer su expediente, `Config::validate()` los
rechaza **por nombre**: un nombre retirado en `providers.enabled` —o una tabla
`[providers.<nombre>]`— produce un error de configuración explícito
(`unknown enabled provider`) en lugar de una fuente omitida en silencio. No queda
ficha que completar ni que descartar; ver «Viabilidad y coste» para el motivo de
mercado.

**Dos puertas cerradas, verificadas por tests.** La retirada es de extremo a
extremo, no sólo de registro: `retired_providers_are_absent_and_refused_by_validate`
y `retired_provider_canary_stops_at_the_registry_door` comprueban que ni siquiera
una configuración construida a mano que liste un nombre retirado llega a
construir una fuente (`UnknownProvider`), y la superficie de UI ya no expone
selector, diálogo de key ni claves retiradas en el catálogo de mensajes.

El adaptador HTML se retiró del registro y de `providers/`: era un stub de 73
líneas sin endpoint, cliente HTTP ni parseo, y no hay API de búsqueda web
disponible (sólo una Instant Answer API, que no devuelve resultados web). SearXNG
autohospedado también se retiró: era agregador de motores upstream, no una fuente
con API oficial propia, y quedaba fuera de la política de fuentes que AMATL sirve
de fábrica. Mantenerlos listados como providers era engañoso; ver «Viabilidad y
coste» más abajo.

La redacción de credenciales en URLs sigue cubierta por el adapter que queda con
credencial: `sanitized_url()` se prueba en `providers/marginalia.rs:495-496`
sobre una URL con `API-Key` en query, de modo que retirar las fuentes de pago no dejó esa
propiedad sin cobertura. La selección de fuentes exige `approved()` y, al
fallar, registra `provider_not_approved` y no llama a la fuente
(`config/providers.rs`, `service.rs:2086-2094`).

## Ficha de aprobación: Marginalia

Esta sección fija la ficha de gobernanza para la única fuente gratuita
verificada. La puerta de activación (`config/mod.rs:108-138`, `approved_on`) exige,
además de `approval_status = "approved"` y un `reviewed_at` vigente (≤ 90 días),
valores no vacíos para `adapter_version`, `reviewer`, `terms_url`,
`terms_version_or_date`, `allowed_access_method`, `plan_or_contract`,
`rate_limit`, `cost_model`, `data_handling_notes` y `operational_risk`. Una
ficha que omita `plan_or_contract` o `rate_limit` **no aprueba**, aunque el
resto esté completo. `adapter_version` debe coincidir con el valor registrado en
`ProviderRegistry` (`marginalia-v1`), no un literal genérico.

### Marginalia — aprobable

El adapter está implementado y probado (`providers/marginalia.rs`): consulta
`api2.marginalia-search.com/search` (el endpoint `api.marginalia.nu` original
está deprecado), envía la clave en el header `API-Key` y traduce `site:` de
forma nativa. Antes de aprobar o habilitar uso productivo, el operador debe
tener una key **genuina, vigente y emitida para su cuenta por Marginalia**. Una
variable no vacía, un placeholder, una key compartida o una key pública de
ejemplo no prueban autenticación ni autorización. Sólo después se completan
`reviewer`, `reviewed_at` y `approval_status` con evidencia real:

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

- `credential_env` sólo referencia `MARGINALIA_API_KEY`; el valor nunca entra
  en TOML, Git, logs ni evidencia de aprobación. Su ciclo de vida (custodia,
  rotación y revocación) sigue `docs/security/secrets.md`.
- El revisor debe registrar la fecha y la fuente de emisión de la key en el
  sistema de secretos o expediente privado, no en este repositorio. El canario
  `provider-canary marginalia` debe devolver resultados utilizables antes de
  habilitar tráfico normal. Un `429` sólo indica límite de cuota/QPM; no valida
  que la key sea auténtica.
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
| Fuentes de pago (**retiradas del producto**) | De pago; no se contratan por política del operador | Sí | Sí, oficial | Índice propio |
| SearXNG autohospedado (**retirado del producto**) | **0**, sin cuota | No | Sí, la de tu instancia (`format=json`, desactivado por defecto) | Agrega Google, Bing, engine-a y otros |
| Marginalia (la única servida) | **0** para uso no comercial (CC-BY-NC-SA) | Sí; para producción requiere una key genuina emitida al operador y una cuota verificable. Una key pública/de ejemplo no es una credencial productiva. | Sí, oficial (`api2.marginalia-search.com`) | Índice propio pequeño, orientado a web independiente |

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

### Por qué se retiró SearXNG

SearXNG no tiene índice propio: agrega motores cuyos términos pueden prohibir el
acceso automatizado, así que traslada un problema en lugar de eliminarlo. La
retirada sigue el mismo criterio que la de las fuentes de pago: el producto
publica sólo fuentes con API de búsqueda web propia y gobernables, en vez de
mantener como seleccionable un agregador cuyos términos dependen de cada motor
upstream que el operador elija. `SEARXNG_INSTANCE_URL` y `deploy/searxng/` ya no
forman parte del repositorio ni de la configuración.

### Alcance de red de los providers

El guard anti-SSRF (`validate_resolved_addresses`) protege **Deep**, no el
transporte de providers, que usa `ReqwestTransport` sin validación de dirección.
Una ficha aprobada que apunte a una fuente autohospedada en `127.0.0.1` o en la
red local es por tanto alcanzable. Es coherente con el modelo —los providers los
configura el operador— pero debe ser una decisión consciente: una ficha
aprobada puede apuntar a infraestructura interna.

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
