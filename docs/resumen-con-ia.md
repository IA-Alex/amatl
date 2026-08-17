# Resumen con IA (`answer`)

Capa opcional y explícita sobre Search: sintetiza una respuesta corta y
citada a partir de los resultados que AMATL ya obtuvo por su cuenta. Nunca
reemplaza `search`/`deep` — son y siguen siendo crudos, sin IA de por medio.
Esta función es la única de las tres superficies de AMATL que sale a un
modelo de lenguaje de terceros.

## Qué hace, exactamente

1. Corre una búsqueda normal (mismos providers, mismo ranking que `search`).
   El modelo no participa en este paso ni lo altera.
2. Toma los primeros `max_sources` resultados ya ordenados, con su
   fragmento recortado a `max_source_chars`, y arma un mensaje de texto
   plano numerado.
3. Envía ese mensaje al proveedor configurado (una sola llamada HTTP,
   acotada por `timeout_ms`) pidiéndole que redacte una respuesta corta
   citando `[n]` por cada afirmación.
4. Revisa mecánicamente las citas: una cita a un índice inventado se
   descarta; si la respuesta no cita ningún índice válido, se rechaza
   entera (`AnswerUnavailable`) en vez de devolverse.

El modelo no tiene acceso al núcleo de AMATL — ni a la base de datos, ni a
los providers, ni a nada más allá del texto que se le entrega en ese único
mensaje. No busca y no reordena resultados; el orden ya viene decidido por
AMATL antes de que el modelo se entere de la consulta.

## Habilitarlo

Requiere dos cosas a la vez: `data_policy.inference = "remote_explicit"`
(el mismo interruptor que gobierna cualquier llamada de inferencia remota) y
`[answer]` configurado en `amatl.toml`:

```toml
[data_policy]
inference = "remote_explicit"

[answer]
enabled = true
endpoint = "https://api.deepinfra.com/v1/openai/chat/completions"
model = "deepseek-ai/DeepSeek-V3"
credential_env = "DEEPINFRA_API_KEY"
timeout_ms = 20000
max_sources = 8
max_source_chars = 1200
max_answer_tokens = 700
```

`endpoint` acepta cualquier API de chat-completions compatible con OpenAI
(`{"model":…,"messages":[…]}` → `choices[0].message.content`) — DeepInfra es
el proveedor de referencia, pero no es el único válido.

**La clave nunca va en `amatl.toml`.** `credential_env` solo nombra la
variable de entorno que la contiene (`DEEPINFRA_API_KEY` en el ejemplo); se
exporta en la terminal antes de arrancar el servidor:

```bash
export DEEPINFRA_API_KEY="tu-clave"
```

Sin esa variable presente y con formato válido al arrancar, el servidor
sigue funcionando normal — `search`/`deep` no se ven afectados — pero
`answer` queda inactiva hasta el próximo reinicio con la clave puesta.

## Cómo se usa

Tres superficies, la misma función:

| Superficie | Cómo |
|---|---|
| Web | Botón "Resumen con IA" junto a Buscar/Analizar evidencia. Visible siempre, atenuado si no está disponible; hacer clic en ese estado no dispara ninguna petición, solo explica qué falta. |
| HTTP | `POST /answer` con `{"q": "consulta"}`, mismo contrato de auth que `/deep` (`Scope::Deep`). |
| MCP | Herramienta `answer`, autorizable por credencial igual que el resto de las herramientas MCP. |

## Verificar el estado sin tocar la clave

`GET /status` expone un bloque `answer` de solo lectura, sin nunca incluir el
secreto:

```json
"answer": {
  "enabled": false,
  "configured": true,
  "available": false,
  "model": "deepseek-ai/DeepSeek-V3",
  "endpoint": "https://api.deepinfra.com/v1/openai/chat/completions"
}
```

- `enabled`: `answer.enabled` en el archivo ahora mismo — lo que refleja y
  controla el interruptor de la web (ver abajo).
- `configured`: si `endpoint` y `model` están puestos en el archivo,
  **independiente de `enabled`** — a propósito, para poder seguir mostrando
  con qué proveedor/modelo quedaría el interruptor aunque esté apagado.
- `available`: si de verdad se puede usar ahora mismo (`enabled` **y**
  `configured` **y** la credencial estaba presente al arrancar). La UI se
  guía por este campo para el botón, no por `enabled` ni `configured` solos.

En la interfaz web esto se ve en el panel "Estado del servicio" →
"Configuración de IA" (colapsable, solo con Modelo/Endpoint/Estado — nunca
la clave, visible mientras `configured` sea `true` sin importar `enabled`),
y como aviso corto debajo de los botones de búsqueda cuando `enabled` es
`true` pero `available` es `false`.

## Activar o desactivar desde la web

`POST /answer/enabled` con `{"enabled": true|false}` — requiere permiso de
administrador (`Scope::Admin`, igual que `/reload`). Es la **única**
mutación de configuración que un proceso de AMATL hace sobre su propio
archivo: reescribe solo `[answer].enabled` en `amatl.toml` (con
`toml_edit`, preservando cada comentario del archivo) y aplica el cambio sin
reiniciar, reusando el mismo mecanismo que `/reload`.

Antes de escribir nada, valida en memoria que el resultado pasaría
`Config::validate` — si activarlo dejaría `answer` sin `endpoint`/`model`
válidos, la petición falla con `configuration_invalid` **sin tocar el
archivo**. Nunca deja el archivo en un estado que el proceso corriendo no
haya aceptado ya.

En la web, el interruptor vive dentro de "Configuración de IA", junto a
Modelo/Endpoint/Estado.

## Límites deliberados

- **Provider, modelo, endpoint y la clave no se editan desde la web** — solo
  `enabled` es alcanzable desde ahí. El resto sigue siendo edición manual de
  `amatl.toml` + `/reload` (o el propio interruptor no serviría de mucho:
  necesita esos campos ya puestos para tener algo que activar).
- **Una sola llamada por consulta**, sin reintentos ocultos dentro de
  `answer` mismo — si falla, falla como `AnswerUnavailable`, tipado y
  reportado igual que cualquier otro error del catálogo.
- **No reduce la cobertura de `search`/`deep`.** Son caminos totalmente
  independientes; una falla de `answer` nunca degrada los otros dos.

## Paleta y estilo

Toda superficie visual de esta función (tarjeta de respuesta, aviso,
panel de configuración) usa exclusivamente los tokens documentados en
[identidad visual](identidad-visual.md) — nada de color nuevo se
introdujo para esta función.
