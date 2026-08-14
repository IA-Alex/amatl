# Operación de AMATL

## Arranque seguro

1. Copia `amatl.example.toml` a una ruta local no versionada y valida con
   `amatl --config-file RUTA config`.
2. Genera el bearer fuera del TOML:

   ```bash
   export AMATL_SERVER_TOKEN="$(openssl rand -hex 32)"
   amatl --config-file amatl.toml serve
   ```

3. El default escucha `127.0.0.1:8080`. Para bind no-loopback configura rutas
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

`/health` sólo prueba que router/proceso responden y devuelve
`{"schema_version":"1","status":"ok"}`. No consulta red, providers, SQLite ni
credenciales. `amatl doctor` carga toda la configuración, lista providers,
comprueba SQLite/migración si procede, reporta telemetría y muestra preparación
de token/TLS. Un `doctor` degradado puede terminar con código 0 porque es un
reporte; el operador debe leer cada línea.

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
