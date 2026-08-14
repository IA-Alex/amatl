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
{"schema_version":"1","status":"ok","storage_ok":true,"sources_available":1}
```

El cuerpo es agregado a propósito. Nombres de fuentes, códigos de error y rutas
describen el despliegue y sólo aparecen en `GET /status`, que exige el scope
`read`. La persistencia desactivada cuenta como sana; habilitada pero no
disponible, no.

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
| `search` | `POST/GET /search` |
| `deep` | `POST/GET /deep` |
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
| `amatl_source_available{source}` | gauge | 1 si la fuente declarada está disponible |
| `amatl_source_success_rate{source}`, `amatl_source_latency_ms{source}` | gauge | valor observado por fuente; sólo aparecen con muestras |
| `amatl_cache_hits_total{cache}`, `amatl_cache_misses_total{cache}`, `amatl_cache_hit_rate{cache}` | counter/gauge | reuso de la caché de búsqueda y de documentos |
| `amatl_storage_available` | gauge | 1 si la persistencia local es usable |
| `amatl_source_circuit_open{source}` | gauge | 1 mientras el circuito de esa fuente está abierto |

Los contadores son monótonos y se reinician con el proceso; las cuantías de
latencia describen sólo la ventana retenida. Los nombres de fuente aparecen
como valores de etiqueta: si esos nombres son sensibles en tu despliegue, no
expongas el puerto fuera del host.

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
el nombre no está habilitado, la ficha de gobernanza no está aprobada/vigente o
falta la variable de credencial declarada. DuckDuckGo HTML sigue bloqueado. No
uses consultas sensibles: el resultado JSON se conserva en el log del operador.

```bash
amatl --config-file amatl.toml \
  provider-canary brave "rust programming language" --json
```

En GitHub, el workflow manual `provider-canary` usa el environment homónimo para
aprobación humana. Requiere el secreto `AMATL_CANARY_CONFIG` con el TOML completo
ya aprobado y `BRAVE_API_KEY` o `MOJEEK_API_KEY`; nunca se ejecuta en `push` ni
en pull requests.

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
