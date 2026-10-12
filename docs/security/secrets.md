# Gestión de secretos

## Fuentes permitidas

Los secretos entran por dos vías sancionadas, y ninguna otra:

1. **Variables de entorno** (vía por defecto). `BRAVE_API_KEY` y
   `MOJEEK_API_KEY` son los defaults de los adapters; el servidor lee la variable
   nombrada por `server.token_env`, cuyo default es `AMATL_SERVER_TOKEN`. El
   archivo `amatl.toml` sólo puede contener el **nombre** de la variable, nunca
   su valor.
2. **Inyección efímera en memoria** (vía admin explícita). Un operador con
   scope `admin` puede insertar la key de un provider desde la UI
   (`POST /providers/{name}/credential`). La key vive sólo en la RAM del
   proceso, se valida con un canario live antes de aceptarse y se erradica con
   `DELETE` o al morir el proceso. Ver «Credenciales efímeras de provider».

Queda prohibido incluir tokens, claves, passwords, cookies o cabeceras de
autorización en código, commits, configuración TOML, fixtures, logs, telemetría,
mensajes de error o URLs visibles. La caché y SQLite tampoco son almacenes de
secretos.

## Credenciales de providers

Una variable no vacía no equivale a una credencial válida. Para un provider
externo como Marginalia, la key debe ser genuina, vigente, emitida para el
operador y tener una cuota/contrato que autorice el uso previsto. No se usan
placeholders, valores compartidos ni claves públicas de ejemplo para tráfico
productivo. La ficha de procedencia y aprobación se mantiene fuera del
repositorio en el sistema de secretos del operador.

Aplica mínimo privilegio: entrega la key sólo al proceso que la necesita, evita
`set -x`, `printenv` y comandos que la incluyan en argumentos, y revócala en el
portal del provider si se sospecha exposición. Para una prueba real, el
`provider-canary` debe producir resultados utilizables; un error `429` indica
cuota o QPM y no certifica autenticación. Estas reglas siguen los principios de
ciclo de vida, control de acceso y minimización de exposición de la [OWASP
Secrets Management Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/Secrets_Management_Cheat_Sheet.html).

## Credenciales efímeras de provider

Para una fuente como Marginalia, el operador puede insertar la API key desde la
UI en vez de exportarla. El diseño es deliberadamente acotado:

- **Entrada:** `POST /providers/{name}/credential` exige scope `admin` y recibe
  la key en el cuerpo JSON (nunca en la URL). `amatl.toml` no cambia.
- **Validación previa:** la key se prueba con un canario live antes de
  comprometerla; una key inválida, vencida o mal emitida se rechaza sin
  guardarse. Bajo `data_policy` que niega egress, el canario falla cerrado y no
  se almacena nada.
- **Almacenamiento:** sólo en RAM del proceso, en un mapa `Zeroizing` que borra
  el buffer de la key al reemplazarla, al liquidarla o al morir el proceso. No
  se escribe en disco, TOML, entorno, logs ni auditoría; `/status` y
  `/providers` exponen únicamente la procedencia (`credential_source`:
  `none`/`ephemeral`/`environment`), nunca el valor.
- **Erradicación:** `DELETE /providers/{name}/credential` (idempotente, scope
  `admin`) la elimina de la memoria y el siguiente Search la ve ausente. La
  UI sólo habilita «Liquidar» cuando `credential_source = "ephemeral"`: si la
  key viene de la variable de entorno declarada en `credential_env`, la UI lo
  indica y no ofrece borrarla, porque no está bajo su control.
- **Auditoría:** cada inserción, liquidación o rechazo emite un evento
  (`provider_credential_inserted`/`liquidated`/`rejected`) con la identidad y
  el provider, nunca la key.
- **Riesgo residual:** la RAM puede ir a swap y no se usa `mlock`; el `String`
  transitorio del header de una request en vuelo no se zeroiza. Para
  aislamiento verificable del entorno, añade sandbox/swap cifrado del host.

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

El formateador JSON sustituye por `[redacted]` los campos con nombres sensibles
y serializa saltos de línea dentro de una sola entrada. Los eventos HTTP de
seguridad no registran valores de Host, Origin ni Authorization. Las pruebas
cubren el formateador, un rechazo autenticado y el error TLS del transporte de
providers. Riesgo residual: un adapter futuro todavía podría introducir un
secreto dentro de un campo con nombre no sensible o de un mensaje libre; la
revisión de código sigue siendo obligatoria.

El proceso sólo publica eventos cuyos targets pertenecen a AMATL. Los targets
de `rmcp`, `hyper`, `reqwest` y otras dependencias se descartan incluso con un
`RUST_LOG` amplio, ya que no están bajo el contrato de redacción y pueden
contener argumentos. Los eventos SSRF omiten URL, host, query y direcciones; el
`request_id` generado por el servidor permite correlacionarlos sin esos datos.

## Respuesta a una fuga

Revoca o rota primero el secreto en su autoridad (provider o servidor), detén
procesos con el valor viejo, conserva evidencia ya depurada, busca el valor y
sus variantes en historial Git, CI, logs, artefactos, shells y respaldos, y
elimina o restringe esos datos. Si llegó a Git, rotar es obligatorio: reescribir
historial por sí solo no recupera confidencialidad. Reporta el incidente por el
canal privado y el contacto verificado definidos por el propietario en
`SECURITY.md`.
