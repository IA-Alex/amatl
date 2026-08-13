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
4. Verifica `/health` sin token y `/search` con bearer. No expongas el token en
   query strings.

Rotación: genera un token nuevo, drena/detén el listener, reemplaza la variable,
reinicia, actualiza clientes y verifica que el anterior devuelve 401. AMATL no
soporta dos tokens simultáneos.

## Salud y diagnóstico

`/health` sólo prueba que router/proceso responden y devuelve
`{"schema_version":"1","status":"ok"}`. No consulta red, providers, SQLite ni
credenciales. `amatl doctor` carga toda la configuración, lista providers,
comprueba SQLite/migración si procede, reporta telemetría y muestra preparación
de token/TLS. Un `doctor` degradado puede terminar con código 0 porque es un
reporte; el operador debe leer cada línea.

## Estados y degradaciones

- `success`: ejecución sana según el contrato.
- `partial_success`: hay resultados útiles, pero un provider fue parcial/falló o
  existe degradación relevante; CLI devuelve 0.
- `failure`: no hay resultado útil; `amatl search` devuelve 1.

`warning` corresponde a interpretación recuperable de Query. `error` identifica
una operación/frontera fallida. `degradation` conserva operación con menor
capacidad. Deep puede producir documentos `superficial` sin extracción o ningún
documento para una URL bloqueada sin invalidar Search.

## SQLite y caché

Con `persistence.enabled = false`, Search y Deep funcionan sin persistencia;
telemetría vive en memoria y reinicia en Bootstrap. Habilitar caché/telemetría
persistente exige habilitar SQLite. Si la base falla al abrir,
`AmatlService` continúa sin storage. `doctor` revela la diferencia.

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
ni correlation IDs, por lo que no debe atribuirse esa capacidad.
