# Operación de AMATL

## Arranque seguro

1. Copia `amatl.example.toml` a una ruta local no versionada y valida con
   `amatl --config-file RUTA config`.
2. Genera el bearer fuera del TOML:

   ```bash
   export AMATL_SERVER_TOKEN="$(openssl rand -hex 32)"
   amatl --config-file amatl.toml serve
   ```

3. El default escucha `127.0.0.1:8080`. `amatl serve --bind` y `--port` lo
   sobrescriben sólo para ese proceso, y `--json` imprime una línea con el
   listener efectivo (bind, puerto, TLS, autenticación, archivo de
   configuración y superficies) antes de escuchar; `amatl mcp serve` acepta las
   mismas opciones. Para bind no-loopback configura rutas
   reales a certificado y clave TLS, conserva `no_auth = false`, restringe
   hosts/origins y ajusta firewall. La configuración rechaza bind remoto sin TLS
   y token.
4. Verifica `/health` sin token y `/search` mediante POST JSON con bearer. La UI
   mantiene el token sólo en memoria de la página y lo envía exclusivamente en
   `Authorization`; no expongas consultas ni tokens en query strings.

### Proceso de larga duración

`amatl serve` corre en primer plano por diseño; para una ronda de pruebas que
sobreviva al cierre de la terminal, despáchalo en segundo plano desde el
binario ya compilado en vez de `cargo run`:

```bash
nohup ./target/release/amatl serve > amatl.log 2>&1 & disown
```

Esto desprende el proceso de la terminal, pero **no** lo convierte en un
servicio del sistema: un apagado o reinicio de la máquina sí lo mata, y hay
que relanzarlo a mano (no arranca solo sin una unidad `systemd` propia, que
no forma parte de esta distribución). Verificá que sigue vivo con
`pgrep -af 'target/release/amatl'` antes de asumir que una prueba tardía va a
encontrar el servidor arriba.

`target/` es la salida de compilación local, no el almacenamiento operativo:
concentra binarios y cachés de Cargo y puede crecer a decenas de GiB tras
tests, lint y benchmarks. `cargo clean` lo puede eliminar para recuperar
espacio, pero borra también `target/release/amatl`; detén primero cualquier
proceso iniciado desde esa ruta y ejecuta `cargo build --release` antes de
volver a usar el comando `nohup` anterior.

Importante no confundir esto con la persistencia de datos: `persistence`
escribe en `amatl.sqlite3`, un archivo en disco independiente del proceso.
Historial y documentos guardados sobreviven a que el proceso se caiga y lo
vuelvas a levantar — sólo se pierden si borrás el archivo o si
`persistence.enabled = false`. Reiniciar el servidor nunca vacía el
historial.

En la UI, `Buscar` llama a `/search` y `Analizar evidencia` llama a `/deep` con
la misma consulta, filtros y bearer. La vista Deep despliega fragmentos,
procedencia y resultado de verificación local de rango/hash. Requiere que los
providers y el fetch de red estén permitidos y que el extractor pueda producir
texto; con perfil `isolated` se mostrará una degradación sin documentos porque
la red se niega antes de DNS. No existe selector de archivos: usa `amatl ingest`
para evidencia local.

La UI muestra además tres paneles laterales que consumen superficies locales y
sólo aparecen cuando el servidor responde a ellas con el bearer cargado:
**Estado del servicio** (`GET /status`), **Historial de búsquedas**
(`GET/DELETE /history`, `DELETE /history/{id}`) y **Documentos guardados**
(`GET/POST /saved`, `DELETE /saved/{id}`). Historial y guardados requieren
`persistence.enabled`; sin persistencia esas rutas responden
`storage_unavailable` (503) y los paneles quedan ocultos. El historial registra
la consulta ejecutada, la superficie de origen y los totales; se borra por
entrada o completo desde la propia UI, y nunca sale de la máquina. La
paginación de resultados es exclusivamente del servidor: la UI envía `page` y
`page_size` y no vuelve a recortar la lista recibida.

### Perfil aislado para ejercicios confidenciales

No requiere API key. Configura:

```toml
[data_policy]
profile = "isolated"
egress = "deny"
inference = "local_only" # o "disabled"

[providers]
enabled = []
```

Después ejecuta `amatl config` o `amatl doctor` y confirma
`network_egress_allowed = false` y `remote_inference_allowed = false`. El
arranque falla si se combina con bind no-loopback, renderer, provider real o
inferencia remota. Los intentos directos por Deep/MCP fallan con
`egress_denied`; no necesitan ni deben recibir credenciales.

Para aislamiento del ejercicio completo, ejecuta también el cliente MCP y toda
inferencia en el mismo entorno local, corta egress en firewall/container y
revisa el ejecutable extractor. La política de AMATL no controla procesos
terceros ni puede recuperar una consulta que ya fue enviada a un servicio cloud.

Rotación: genera un token nuevo, drena/detén el listener, reemplaza la variable,
reinicia, actualiza clientes y verifica que el anterior devuelve 401. AMATL no
soporta dos tokens simultáneos.

## Ingestión local

La ingestión acepta un archivo regular explícito y nunca está disponible desde
el listener HTTP/MCP:

```bash
amatl ingest ./evidencia.md --query "hallazgos críticos" --json
```

No necesita provider ni API key. Texto, Markdown, HTML, JSON/JSONL, CSV y código
se procesan dentro de AMATL incluso con perfil aislado. PDF requiere
`pdftotext` en `PATH` y sólo se ejecuta cuando la política efectiva permite
egress; con `isolated`/`deny` falla antes de crear el proceso. La salida JSON
incluye URI local absoluta y cuerpo extraído: redirígela sólo a destinos
autorizados. Consulta [el contrato y los límites](ingestion-local.md).

## Salud y diagnóstico

`/health` es la sonda de **liveness**: sólo prueba que router/proceso responden
y devuelve `{"schema_version":"1","status":"ok"}` con `200` siempre. No consulta
red, providers, SQLite ni credenciales, y eso es deliberado: un orquestador la
usa para decidir si reinicia el proceso, decisión que no debe depender de que un
provider esté alcanzable.

`/ready` es la sonda de **readiness**, también pública y sin token. Devuelve
`200` cuando la instancia puede servir tráfico útil y `503` cuando está
degradada, de modo que un balanceador pueda drenarla sin interpretar el cuerpo:

```json
{"schema_version":"1","status":"ok","storage_ok":true,"sources_available":1,"sources_usable":1}
```

El cuerpo es agregado a propósito. Nombres de fuentes, códigos de error y rutas
describen el despliegue y sólo aparecen en `GET /status`, que exige el scope
`read`. La persistencia desactivada cuenta como sana; habilitada pero no
disponible, no. Una fuente declarada pero no habilitada es una decisión de
configuración, no una incidencia: sólo una fuente **habilitada** que esté
aprobada, disponible y con el circuito cerrado mantiene la instancia lista.

La regla completa, porque el caso ambiguo es justamente el de los orígenes
declarados y deshabilitados:

| Orígenes **habilitados** | Estado de ésos | `/ready` |
|---|---|---|
| Uno o más | Al menos uno aprobado, disponible y con el circuito cerrado | `200` |
| Uno o más | Ninguno utilizable (no aprobado, no disponible o circuito abierto) | `503` |
| Ninguno —sólo declarados y deshabilitados— | — | `503` |

Dicho de otro modo: «un origen habilitado y utilizable, con el resto declarado
y deshabilitado» responde `200`; «ningún origen habilitado» responde `503`,
porque la instancia no puede buscar. Declarar y deshabilitar orígenes no es una
incidencia, y tampoco es capacidad.

`sources_available` cuenta los orígenes cuyo estado de superficie es
`available` —el mismo valor que `GET /status` publica por fuente— y **no** es el
predicado de readiness: un origen habilitado con el circuito abierto sigue
contando ahí mientras la instancia responde `503`, de modo que el campo puede
valer `1` en una respuesta degradada. Automatice sobre el código HTTP o sobre
`status`, nunca sobre ese contador; cambiar su significado exige revisar los
consumidores y versionar el contrato.

`sources_usable` es el contador que sí responde «¿puede este origen servir
ahora mismo?»: aplica el mismo filtro de superficie que `sources_available` y
además descarta los orígenes con el circuito **abierto** (en cooldown). Nunca
supera a `sources_available`, así que `sources_available - sources_usable` es
exactamente cuántos orígenes está saltando un breaker abierto. Es un campo
aditivo: no cambia `sources_available`, que sigue siendo la misma cantidad que
publica el gauge `amatl_sources_available`. Una automatización que quiera evitar
el falso positivo de `sources_available` debe leer `sources_usable`; la sonda de
readiness sigue siendo el código HTTP o `status`.

Ese contador y el gauge `amatl_sources_available` de `/metrics` son la misma
cantidad publicada en dos superficies **en un despliegue real**.
`amatl_sources_declared` cuenta además las fuentes **deshabilitadas**, que
reportan `unavailable` con `provider_disabled`: por eso `declared − available` no
es un conteo de incidentes —en un despliegue de un solo origen vale uno por
configuración— y alertar sobre esa diferencia avisa de una decisión del operador,
no de una avería. Para incidentes use `amatl_sources_circuit_open` y los gauges de
peor caso.

Con `amatl serve --mock` las dos superficies **discrepan**, y hay que saber cuál
mirar: `/ready` informa las fuentes simuladas con las que el servicio busca de
verdad —en el despliegue mock aislado, `200` con `"sources_available":2`— mientras
`/metrics` sigue describiendo las fuentes declaradas reales, que en ese modo no
sirven tráfico (`amatl_sources_declared 2`, `amatl_sources_available 0`). Un
despliegue mock sano puede por tanto publicar `amatl_sources_available 0`, así que
no automatice una alerta de caída sobre ese gauge sin excluir el modo mock. La
divergencia está fijada por
`mock_deployments_disagree_between_ready_and_the_source_gauges`; alinearla exige
cambiar el valor de una métrica publicada, y es una decisión del propietario, no
un arreglo de documentación.

`GET /status` y la herramienta MCP `status` reportan además el estado de
circuito por fuente. `amatl doctor` carga toda la configuración, lista providers,
comprueba SQLite/migración si procede, reporta telemetría y muestra preparación
de token/TLS. Un `doctor` degradado puede terminar con código 0 porque es un
reporte; el operador debe leer cada línea.

La CLI usa el mismo catálogo de códigos que API y MCP: ante un fallo con código
imprime `error_code=<código> message=<mensaje>` en stderr, y ante una búsqueda
con `status = failure` imprime los códigos compuestos que ya trae la respuesta
(`no_available_provider`, `no_usable_results`). stdout conserva sólo la salida
del comando, de modo que `--json` sigue siendo parseable.

Toda respuesta de aplicación incluye `X-Request-ID`. AMATL genera uno nuevo en
el borde HTTP y no confía en un header homónimo recibido. Conserva ese valor al
reportar un incidente: los eventos HTTP, routing y SSRF ejecutados dentro de la
solicitud comparten el mismo contexto. Ese identificador también viaja hacia
adentro: cada llamada saliente a un provider (`amatl::providers`) y cada fetch
del pipeline Deep (`amatl::fetch`) se ejecuta dentro de un span que lo declara,
de modo que una sola búsqueda en logs reconstruye la solicitud completa. El
identificador nunca se envía al provider ni al origen remoto: sólo etiqueta la
traza local. Las superficies MCP generan el suyo por llamada a herramienta.

`GET /status` (con bearer) resume el estado operativo en JSON: disponibilidad y
valor observado por fuente, estado de la persistencia local —versión de
migración, entradas de historial y documentos guardados— y efectividad de las
cachés. Devuelve `status: degraded` cuando alguna fuente declarada no está
disponible o cuando la persistencia está habilitada pero no es usable.

### Credenciales, scopes y herramientas

Sin `[[server.clients]]`, el token de `server.token_env` sigue siendo el único
cliente (`default`) con todas las capacidades: nada cambia para un despliegue
de un solo operador. Declarar clientes con nombre reparte capacidad:

| Scope | Rutas |
|---|---|
| `search` | `POST/GET /search`, `GET /search/batch` |
| `deep` | `POST /deep` |
| `read` | `/providers`, `/status`, lectura de `/history` y `/saved` |
| `write` | escritura y borrado en `/history` y `/saved` |
| `admin` | `/reload`, `/security-events` |
| `mcp` | la superficie `/mcp`; `tools` la acota herramienta por herramienta |

El secreto nunca se escribe en configuración: se declara `token_env` (variable
de entorno) o `token_sha256` (digest). La comparación es sobre digests y en
tiempo constante, así que una fuga del archivo no entrega una credencial usable.
`expires_at` caduca la credencial sin editar nada más, y `POST /reload` o
`SIGHUP` aplican altas, bajas y rotaciones sin reiniciar el proceso: la ventana
de rate limit **no** se reinicia con la recarga.

Un token sin capacidad para una ruta recibe `403 scope_denied`, distinto de
`401 unauthorized`; ambos quedan en la auditoría. En MCP, cada herramienta
comprueba la lista `tools` de la identidad autenticada —no un encabezado del
cliente— así que puedes conceder `search` y negar `fetch`, que es la más
sensible, sin apagar el egress para todos.

### Bitácora de seguridad

Con persistencia activa, cada rechazo del borde HTTP (`unauthorized`,
`scope_denied`, `credential_expired`, `invalid_host`, `invalid_origin`,
`rate_limited`, `headers_too_large`, `body_too_large`, `request_timeout`) se
guarda además en SQLite y se consulta con `GET /security-events` (scope `admin`)
o `amatl db security-events`. La escritura es en segundo plano y acotada: bajo
una avalancha se descartan eventos y el contador
`amatl_audit_events_dropped_total` lo declara, en vez de convertir la auditoría
en la caída. Sin persistencia, el endpoint responde `storage_unavailable` y los
logs siguen siendo el único registro. Retención:
`persistence.audit_retention_days` (90 días por defecto).

### Cortesía de crawl y robots.txt

Deep distingue dos cosas: una URL que el usuario pidió (resultado de Search) se
recupera como user-agent, sin consultar `robots.txt`; una URL que AMATL
descubrió siguiendo un enlace a profundidad ≥ 1 es crawl y sí pasa por el
`robots.txt` del origen. El grupo específico `amatl` gana sobre `*`, se aplica
longest-match con `Allow` desempatando, y se respeta `Crawl-delay` hasta 5 s y
siempre dentro del deadline de Deep. Un 4xx (incluido 404) permite el crawl; un
5xx o un origen inalcanzable lo detiene. Cada rechazo aparece como degradación
(`robots_disallowed`, `robots_unavailable`, `robots_crawl_delay_too_long`).
`deep.respect_robots = false` lo desactiva, y es decisión explícita del
operador.

### Recarga en caliente

Alta, baja o reaprobación de una fuente no requiere reinicio: edita el archivo
de configuración y ejecuta `POST /reload` con bearer, o envía `SIGHUP` al
proceso en Unix. La recarga construye un servicio nuevo completo y lo
intercambia; las solicitudes en curso terminan con la configuración con la que
empezaron y la siguiente ya usa la nueva. Una configuración inválida se rechaza
antes de reemplazar nada y el servicio en ejecución queda intacto. La respuesta
enumera fuentes declaradas, habilitadas y registradas, y el backend de
inferencia resultante. La superficie MCP comparte el mismo handle, así que
también ve la recarga.

Dos compuertas se aplican en cada búsqueda, no sólo al arrancar:

- **Gobernanza.** Una fuente habilitada cuyo registro de aprobación esté
  incompleto o vencido no se construye ni se llama; la respuesta incluye una
  degradación `provider_not_approved` con el nombre de la fuente. Mantener el
  nombre en `providers.enabled` no basta para enviar tráfico.
- **Circuito.** Tras `circuit_breaker.failure_threshold` fallos consecutivos la
  fuente se salta durante `circuit_breaker.open_seconds` (degradación
  `provider_circuit_open`), luego se permite una sonda. El estado se persiste
  cuando SQLite está activo, así que un reinicio dentro de la ventana no vuelve
  a gastar presupuesto redescubriendo la caída. `amatl db circuits` lo muestra y
  `--reset` lo cierra.

### Métricas

`GET /metrics` es público, como `/health`, y expone formato de exposición
Prometheus 0.0.4:

| Métrica | Tipo | Significado |
|---|---|---|
| `amatl_search_requests_total`, `amatl_deep_requests_total` | counter | solicitudes recibidas por superficie |
| `amatl_search_errors_total`, `amatl_deep_errors_total` | counter | solicitudes que terminaron en error |
| `amatl_rate_limited_total`, `amatl_unauthorized_total`, `amatl_request_timeout_total` | counter | rechazos del borde HTTP |
| `amatl_search_latency_ms`, `amatl_deep_latency_ms` | gauge | p50/p95/p99 sobre las últimas 1024 solicitudes de esa superficie |
| `amatl_search_latency_samples`, `amatl_deep_latency_samples` | gauge | muestras retenidas en la ventana |
| `amatl_sources_declared` | gauge | fuentes que declara el despliegue, **incluidas** las deshabilitadas |
| `amatl_sources_available` | gauge | fuentes declaradas con superficie `available`; es la misma cantidad que `sources_available` en `/ready` |
| `amatl_sources_circuit_open` | gauge | fuentes declaradas en cooldown de circuito |
| `amatl_sources_success_rate_min`, `amatl_sources_latency_ms_max` | gauge | peor valor observado entre las fuentes con muestras; sólo aparecen con muestras |
| `amatl_cache_hits_total{cache}`, `amatl_cache_misses_total{cache}`, `amatl_cache_hit_rate{cache}` | counter/gauge | reuso de la caché de búsqueda y de documentos |
| `amatl_storage_available` | gauge | 1 si la persistencia local es usable |

Los contadores son monótonos y se reinician con el proceso; las cuantías de
latencia describen sólo la ventana retenida. `/metrics` es público, así que es
deliberadamente agregado, igual que `/ready`: **los nombres de fuente no
aparecen como etiquetas**. Qué fuentes declara y cuáles fallan es información
interna del despliegue y vive en `/status`, que exige el scope `read`. Las
etiquetas `cache` restantes son clases internas de caché
(`provider_search`/`document`), no configuración del operador. El perfil de
tráfico y latencia sí queda visible sin credencial: si eso importa en tu
despliegue, no expongas el puerto fuera del host.

## Estados y degradaciones

- `success`: ejecución sana según el contrato.
- `partial_success`: hay resultados útiles, pero un provider fue parcial/falló o
  existe degradación relevante; CLI devuelve 0.
- `failure`: no hay resultado útil; `amatl search` devuelve 1.

`warning` corresponde a interpretación recuperable de Query. `error` identifica
una operación/frontera fallida. `degradation` conserva operación con menor
capacidad. Deep puede producir documentos `superficial` sin extracción o ningún
documento para una URL bloqueada sin invalidar Search.

## Canario de provider real

El canario ejecuta exactamente un provider y falla antes de acceder a la red si
el nombre no está habilitado, el provider no está registrado o no admite canario
de red, la ficha de gobernanza no está aprobada/vigente, o falta la variable de
credencial declarada. Para Marginalia, usa sólo una key genuina y vigente
emitida al operador; la presencia de `MARGINALIA_API_KEY` no prueba que sea
válida. No uses consultas sensibles: el resultado JSON se conserva en el log del
operador.

**Un canario que termina en `success` sí generó tráfico saliente**: esa es su
función (un proveedor no se valida sin una llamada real). Lo que no produce
tráfico es el rechazo: cuando la preflight del canario o la validación de
`amatl.toml` fallan, el comando sale con `exit=1` antes de conectar.

Que el canario pueda llegar a la red lo decide **`[data_policy]`, no el
canario**. `allows_network_egress()` es verdadero sólo con un perfil distinto de
`isolated` **y** `egress = "governed"`. Dos consecuencias que cuestan tiempo si
no se conocen:

- **Sin `[data_policy]` no hay red.** Omitir la sección deja `egress` en `deny`
  por defecto, así que el canario responde `provider_not_approved` y nunca llega
  a conectar. No es un problema de ficha ni de instancia: falta la sección.
- **`isolated` no llega a ejecutarse como canario.** `Config::load_optional()`
  aplica `Config::validate()`, que rechaza la combinación «provider habilitado +
  egress denegado» antes de que el subcomando corra
  (`invalid result policy: providers cannot be enabled while network egress is
  denied`), con `error_code=configuration_invalid` y `exit=1`. Por eso el código
  `egress_denied` de la preflight (`validate_provider_canary_with`) no es
  observable desde el CLI: el archivo se rechaza antes de llegar a ella. La
  preflight sigue siendo la defensa en profundidad para cualquier llamador
  embebido en `amatl-core` que no pase por la CLI.

```bash
amatl --config-file amatl.toml \
  provider-canary marginalia "rust" --json
```

Cada rechazo sale con `exit=1` e imprime `error_code=<código>` en stderr. Para
distinguir por qué falló, sin leer el texto libre:

| Situación | `error_code` | ¿Tráfico? |
| --- | --- | --- |
| Nombre no declarado en `providers.enabled` | `configuration_invalid` | no |
| Declarado pero no habilitado | `provider_not_enabled` | no |
| Habilitado pero no registrado en el registry | `provider_not_registered` | no |
| Sin canario de red autorizado para ese provider | `provider_network_blocked` | no |
| Habilitado, ficha incompleta o vencida (>90 días) | `provider_not_approved` | no |
| Falta la variable de credencial (sólo fuentes que exigen key, como Marginalia) | `provider_credential_missing` | no |
| Llegó al proveedor y falló (429, 503, timeout, cuerpo inválido) | el código del upstream (`provider_rate_limit`, `provider_unavailable`, `provider_timeout`, `provider_invalid_response`) | **sí** |

El último renglón es la distinción operativa importante: un fallo del proveedor
después de la llamada **ya consumió** cuota y quedó registrado en el upstream.

**Marginalia es la única fuente servida.** El canario comprueba que la ficha
está aprobada y vigente, que la fuente está registrada y que existe el valor de
`MARGINALIA_API_KEY`; la presencia de la variable no prueba que la key sea
válida. Ejecuta una sola consulta canario y comprueba que el resultado incluya
`marginalia` en `providers_used`. Si devuelve `429`/`QPM Limit Exceeded`, detén
reintentos, espera la ventana de cuota y revisa el plan o la key en el portal del
provider; ese estado no confirma autenticación.

En GitHub, el workflow manual `provider-canary` usa el environment homónimo para
aprobación humana. Requiere el secreto `AMATL_CANARY_CONFIG` con el TOML completo
ya aprobado y el secreto `MARGINALIA_API_KEY`; nunca se ejecuta en `push` ni en
pull requests.

### Comprobar que los nombres retirados siguen cerrados

Con el registro reducido a Marginalia (ver ADR-016, ADR-019 y
`docs/gobernanza-providers.md`), el canario sirve además para verificar que
ningún nombre retirado vuelve por configuración. Los dos casos fallan **antes de
cualquier intento de red**, con `exit=1` y `error_code=configuration_invalid`:

```bash
# providers.enabled = ["unshipped_source"], sin tabla declarada
$ amatl --config-file retired.toml provider-canary unshipped_source rust
invalid result policy: unknown enabled provider 'unshipped_source'

# [providers.unshipped_source] completo y aprobado declarado a mano
$ amatl --config-file retired-declared.toml provider-canary unshipped_source rust
invalid result policy: unknown enabled provider 'unshipped_source'
```

Verificado el 2026-10-26 con una instancia SearXNG viva como control positivo del
mecanismo de retiro (la fuente se retiró después; el registro de gobernanza, ver
`docs/gobernanza-providers.md`). Los tres mensajes de rechazo son deliberadamente
distintos —`provider_not_enabled` para una fuente declarada pero no habilitada,
`provider_not_approved` para una habilitada cuya ficha de gobernanza está
incompleta, y el error de política para un nombre retirado— y **sólo el
`success`** del control positivo generó tráfico; los tres rechazos no conectan.

### Canario de Marginalia en un despliegue real

La única fuente servida exige una key **genuina, vigente y emitida para el
operador** por Marginalia. No uses consultas sensibles: el resultado JSON se
conserva en el log del operador.

```bash
export MARGINALIA_API_KEY=...
# con [data_policy] y la ficha ya configuradas (ver «Canario de provider real»):
amatl --config-file amatl.toml provider-canary marginalia "rust" --json
```

La ficha de gobernanza del provider sigue necesitando aprobación en
`amatl.toml`; un `status: success` con resultados utilizables es la única señal
de que la key y la cuota funcionan.

En el mismo despliegue, `/ready` responde `200` con `"sources_available":1`,
`/status` responde `ok` listando **exactamente una fuente** (`marginalia`), y
`/metrics` publica `amatl_sources_declared 1` / `amatl_sources_available 1`. Al
escribir la tabla `[providers.<nombre>]` hay que reescribir la ficha completa: la
tabla reemplaza el expediente incorporado entero en vez de fusionarse campo a
campo (ver `docs/configuracion.md`).

## SQLite y caché

Con `persistence.enabled = false`, Search y Deep funcionan sin persistencia;
telemetría se comparte durante toda la vida del proceso y reinicia en Bootstrap
solo al reiniciarlo. Habilitar caché/telemetría persistente exige habilitar
SQLite. Si la base falla al abrir, `AmatlService` continúa sin storage, emite un
`warning` operativo y añade `storage_unavailable` a las degradaciones de Search
y Deep; si hubo cuarentena, el mensaje incluye su ruta. `doctor` también revela
la diferencia.

```bash
amatl cache
amatl cache --purge
```

La purga elimina caché de provider y documentos, no telemetría. Consulta
`docs/security/data-retention.md` antes de borrar el archivo SQLite.

### Mantenimiento e historial desde la CLI

Estos comandos requieren `[persistence] enabled = true` y fallan con un mensaje
explícito si no la hay; todo el estado es local:

```bash
amatl history list --limit 20      # búsquedas registradas, más recientes primero
amatl history delete 12            # una entrada
amatl history purge                # todo el historial
amatl saved list                   # documentos guardados
amatl saved show 3                 # payload almacenado de uno
amatl saved delete 3
amatl db health --json             # journal, migración, pool, disco, purga y backups
amatl db backups                   # copias automáticas, de migración y de pre-restauración
amatl db downgrade --to 4          # rollback de esquema, con copia previa
amatl db restore <copia>           # reemplaza la base por una copia
amatl db circuits [--reset]        # estado de los cortacircuitos por fuente
amatl db security-events --json    # bitácora de rechazos, más recientes primero
```

`db downgrade` es destructivo por definición: toma una copia antes de aplicar
los scripts y vuelve a migrar hacia adelante la próxima vez que un binario
nuevo abra la base. Detén los demás procesos AMATL antes de `db restore`.

### Mantenimiento en segundo plano

Con `[persistence] enabled = true`, el servicio lanza una tarea de fondo con dos
cadencias independientes:

| Parámetro | Efecto | 0 significa |
|---|---|---|
| `purge_interval_seconds` | Periodo del ciclo de purga | purga desactivada |
| `auto_backup_interval_seconds` | Periodo del backup automático | — (requiere `auto_backup_enabled`) |
| `auto_backup_max_count` | Copias automáticas retenidas | — |
| `backup_directory` | Destino de las copias automáticas | por defecto, el directorio de la base |

Cada ciclo de purga elimina entradas más antiguas que
`history_retention_days`, `cache_retention_days`,
`document_cache_retention_days`, `audit_retention_days` y la retención de
telemetría; en todas ellas `0` significa *sin límite*, no *purgar todo*.

Los backups automáticos se escriben con `VACUUM INTO`, de modo que la copia es
transaccionalmente consistente y ya está checkpointeada: no lleva `-wal`
asociado y puede restaurarse tal cual. Cada copia se verifica con
`PRAGMA quick_check` abriéndola en solo lectura, y se descarta si no pasa.

`db backups` lista los tres tipos con el mismo comando:

| Nombre | Origen | ¿Rota? |
|---|---|---|
| `<base>-auto-<ts>.sqlite3` | tarea de fondo | sí, por `auto_backup_max_count` |
| `<base>.backup-<ts>.sqlite3` | previa a migración o `db downgrade` | no |
| `<base>.pre-restore-<ts>.sqlite3` | previa a `db restore` | no |

La rotación sólo borra copias automáticas; las de migración y pre-restauración
son puntos de recuperación deliberados y se conservan.

`db health` expone además el espacio libre del sistema de ficheros, el uso en
porcentaje, si el proceso mantiene el lock advisory, los tamaños de la base y su
WAL, y las marcas del último ciclo de purga y del último backup con su resultado
de integridad.

## Runbooks

### Provider caído o no disponible

Ejecuta `amatl providers` y `amatl doctor`; distingue `provider_disabled`,
credencial ausente y gobernanza no aprobada/expirada. No fuerces activación.
Verifica estado/contrato del tercero fuera de AMATL, reduce su prioridad o
retíralo de `providers.enabled`. Search debe degradar si otra fuente aporta
resultado útil.

### Rate limit del provider

Conserva `Retry-After`; no aumentes retries sobre el máximo dos. Reduce tráfico,
revisa cuota/coste y ficha de gobernanza, y espera la ventana del proveedor. El
rate limit HTTP de AMATL es distinto y devuelve 429 con `Retry-After: 60`.

### Budget agotado

Revisa degradación/stop reason (`provider_limit`, `time_exhausted`,
`deadline_near`, fetch/bytes/redirects/cost/subquery). No reintentes en loop.
Ajusta sólo valores válidos con evidencia de benchmark y conserva límites MCP
más estrictos.

### SQLite degradado o corrupto

Confirma con `doctor`. AMATL pone en cuarentena una cabecera/quick-check inválida
y continúa sin storage si `AmatlService` no puede abrir. Conserva el archivo de
cuarentena para análisis, revisa disco/permisos y recrea cachés descartables; no
restaures contenido no confiable dentro de una base nueva.

### Sospecha de secreto filtrado

Rota primero, detén procesos con el valor anterior y sigue
`docs/security/secrets.md`. No adjuntes el valor a logs o issues.

## Logs

`RUST_LOG=amatl=debug` activa decisiones de routing; producción debe usar el
nivel mínimo necesario. stderr redirigido usa JSON; el operador decide destino,
acceso, rotación y retención. AMATL no implementa un almacén durable de auditoría
propio. Los targets de dependencias se excluyen aunque `RUST_LOG` sea amplio,
porque algunas bibliotecas pueden registrar argumentos completos. Los eventos
JSON incluyen la cadena de spans; `http_request` aporta `request_id`, ruta sin
query y dirección obtenida del socket.
