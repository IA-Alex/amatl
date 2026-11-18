# Depuración local de AMATL

Cómo inspeccionar un fallo concreto con un depurador real (DebugMCP sobre
codelldb/lldb) sobre los binarios del workspace, y qué impide hacerlo en hosts
con `ptrace` restringido. No sustituye a `docs/testing.md`: el depurador sirve
para entender *por qué* un caso ya reproducible falla, no para probar
comportamiento.

## Requisitos

- Binarios **con información de depuración**: `cargo build -p amatl-cli` (perfil
  `dev`). Un `--release` sin `debug = true` deja al depurador sin líneas ni
  variables, y el breakpoint no resuelve.
- Un adaptador DAP registrado (aquí, codelldb para Rust) y un cliente MCP que
  exponga las herramientas de DebugMCP.
- `ptrace` permitido para el proceso que lanza el debuggee; en caso contrario,
  ver la sección siguiente.

## La restricción de `ptrace`

El lanzamiento falla con

```text
Cannot launch '<bin>': ptrace failed: Operation not permitted
```

cuando el kernel exige `CAP_SYS_PTRACE` y el proceso no la tiene. En este host
coinciden las dos condiciones:

```bash
sysctl kernel.yama.ptrace_scope      # 2 = sólo CAP_SYS_PTRACE
grep '^CapEff' /proc/self/status     # 0000000000000000
```

Con `ptrace_scope=2` ni el propio usuario puede trazar sus procesos, así que la
única vía sin root es un **user namespace**: dentro de él el proceso recupera
`CAP_SYS_PTRACE` sobre los procesos que ese namespace crea. La alternativa es
`kernel.yama.ptrace_scope=1` (o `0`) con `sysctl`, que exige root y relaja la
protección para todo el sistema.

### Wrapper de user namespace

`~/.local/bin/debugmcp-ns` envuelve al servidor MCP de modo que tanto el
adaptador como el debuggee nazcan dentro del namespace:

```sh
#!/bin/sh
exec unshare -Ur --map-root-user -- "$HOME/.local/bin/debugmcp" "$@"
```

El wrapper no concede privilegio real sobre el host: `--map-root-user` sólo
mapea el uid del usuario a 0 *dentro* del namespace. Aun así, trátalo como una
frontera de confianza y no depures con él binarios que no hayas compilado.

Verificación (debe imprimir una máscara no nula):

```bash
unshare -Ur --map-root-user -- sh -c 'grep ^CapEff /proc/self/status'
# CapEff: 000001ffffffffff
```

`unshare` viene de util-linux y exige que el host permita *user namespaces* no
privilegiados; si `unshare -Ur` falla con `Operation not permitted`, esa
restricción es del kernel y no del depurador.

## Registrar el adaptador

codelldb habla DAP por stdio y no acepta `--stdio`, así que el adaptador se
registra sin argumentos y con `transport: "stdio"` en
`~/.config/debugmcp/config.json`:

```json
{
  "version": 1,
  "adapters": {
    "rust": {
      "command": "/home/<usuario>/.local/share/codelldb/extension/adapter/codelldb",
      "args": [],
      "type": "lldb",
      "extensions": [".rs"],
      "transport": "stdio"
    }
  }
}
```

La configuración no expande `~` ni `$HOME`: las rutas van absolutas y ya
resueltas. El nombre `rust` es el que se pasa después como
`configurationName`. El cliente MCP debe apuntar al wrapper, no al binario
directo:

```json
{ "command": "/home/<usuario>/.local/bin/debugmcp-ns", "args": ["serve", "--stdio"] }
```

## Flujo verificado

Sobre un binario ya compilado en `target/debug/`, con `workingDirectory` en la
raíz del workspace:

1. `add_breakpoint` con `fileFullPath` apuntando al **fuente** y la línea deseada.
2. `start_debugging` con `fileFullPath` del **binario**, `workingDirectory` y
   `configurationName: "rust"`.
3. La respuesta trae `"paused": true`, `currentLine` y el `stackTrace` del
   binario.
4. `list_variable_names` y `get_variables_values` leen el estado del frame.
5. `step_over`/`step_into`/`step_out`, `continue_execution` y `stop_debugging`
   completan la sesión.

Ejemplo verificado en este host con el binario `target/debug/amatl` y un
breakpoint en `crates/amatl-cli/src/main.rs:469` (primera instrucción del `main`,
`init_logging()`):

```text
add_breakpoint   { "fileFullPath": "<repo>/crates/amatl-cli/src/main.rs", "line": 469 }
start_debugging  { "fileFullPath": "<repo>/target/debug/amatl",
                   "workingDirectory": "<repo>", "configurationName": "rust" }

Debug session stopped at breakpoint for: <repo>/target/debug/amatl
  "paused": true,
  "currentLine": 469,
  "currentLineContent": "init_logging();",
```

Los binarios auxiliares del workspace también se pueden depurar, con la salvedad
de que algunos exigen argumentos: `lock_probe` necesita la ruta de la base de
datos como primer argumento (`crates/amatl-core/src/bin/lock_probe.rs:21-23`) y
sin él aborta antes de alcanzar cualquier breakpoint posterior.

## Limitaciones conocidas

- **`evaluate_expression` no evalúa expresiones.** Con el adaptador lldb se
  comporta como intérprete de comandos de LLDB: `2+2` responde
  `'2' is not a valid command`, y `expr 2 + 2` no devuelve un resultado legible.
  Para leer estado, usar `list_variable_names` seguido de `get_variables_values`.
- **La launch automática no envía `args`.** El cliente construye la petición con
  el programa y el directorio de trabajo, sin argumentos, así que un debuggee que
  los exija necesita una configuración propia: define `.vscode/launch.json` en el
  workspace y pasa su `configurationName` a `start_debugging` (no verificado en
  este host).
- **Variables de bloques `async`.** `list_variable_names` puede devolver el
  nombre de primer nivel (por ejemplo `body`) en lugar de los campos internos;
  pide ese nombre y expande desde ahí.
- **`get_debug_status` es la fuente de verdad.** Confirma que la sesión está
  `paused` antes de leer variables: un breakpoint añadido sin sesión activa sólo
  queda registrado.

## Verificación sin cliente

Para comprobar la cadena completa (wrapper → codelldb → binario) sin depender de
un cliente MCP concreto, conduce el servidor por stdio con JSON-RPC:
`initialize`, `tools/list`, `add_breakpoint`, `start_debugging`,
`get_debug_status` y `stop_debugging`. Un `start_debugging` que responda
`"paused": true` con un `stackTrace` del binario confirma que `ptrace` funciona
dentro del namespace; un error de `ptrace` indica que el servidor arrancó **sin**
el wrapper.
