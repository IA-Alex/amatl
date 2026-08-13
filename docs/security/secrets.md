# Gestión de secretos

## Fuentes permitidas

Los secretos sólo entran mediante variables de entorno. `BRAVE_API_KEY` y
`MOJEEK_API_KEY` son los defaults de los adapters; el servidor lee la variable
nombrada por `server.token_env`, cuyo default es `AMATL_SERVER_TOKEN`. El archivo
`amatl.toml` sólo puede contener el **nombre** de la variable, nunca su valor.

Queda prohibido incluir tokens, claves, passwords, cookies o cabeceras de
autorización en código, commits, configuración TOML, fixtures, logs, telemetría,
mensajes de error o URLs visibles. La caché y SQLite tampoco son almacenes de
secretos.

## Token del servidor

El bearer debe tener al menos 32 bytes. Generación local sugerida:

```bash
export AMATL_SERVER_TOKEN="$(openssl rand -hex 32)"
```

Para rotarlo:

1. genera un valor nuevo en un canal seguro;
2. detén nuevas solicitudes;
3. sustituye la variable del proceso o del gestor de secretos;
4. reinicia AMATL y actualiza clientes autorizados;
5. invalida y elimina el valor anterior de shells, unidades y pipelines;
6. verifica con una solicitud autorizada y otra con el token viejo.

AMATL usa un único token activo; no existe periodo de solapamiento ni lista de
revocación. `no_auth = true` sólo es válido en loopback y es exclusivamente para
desarrollo.

## Prevención de exposición

El transporte de providers devuelve errores genéricos. Las URLs de request
tienen una vista que sustituye valores de `api_key`, `key` y `token` por
`[redacted]` (`providers/http.rs:12-29`). SafeFetcher sólo permite `accept`,
`accept-language` y `user-agent`, por lo que no puede reenviar Authorization o
Cookie (`fetch.rs:221-230`). Los errores HTTP publican códigos fijos y no detalles
internos (`amatl-server/src/lib.rs:517-535`).

Riesgo residual: la vista sanitizada no está impuesta por un wrapper central a
futuros adapters y no hay una prueba end-to-end de detección de secretos en
todos los logs.

## Respuesta a una fuga

Revoca o rota primero el secreto en su autoridad (provider o servidor), detén
procesos con el valor viejo, conserva evidencia ya depurada, busca el valor y
sus variantes en historial Git, CI, logs, artefactos, shells y respaldos, y
elimina o restringe esos datos. Si llegó a Git, rotar es obligatorio: reescribir
historial por sí solo no recupera confidencialidad. Reporta el incidente por el
canal privado definido en `SECURITY.md`; dicho canal está pendiente de definición
por el propietario.
