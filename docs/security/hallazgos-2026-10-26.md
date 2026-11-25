# Hallazgos — revisión de seguridad y remediación (2026-10-26)

Base: `ce172d9d` (2026-10-26), árbol de trabajo con cambios sin *commit*.
Alcance: núcleo `amatl-core` (spawn de procesos hijos, validación de configuración
profunda, redacción de logs) y empaquetado; de `amatl-server` solo se mira el
empaquetado, no su código. No se revisó la UI.
Método: lectura de cada sitio de `Command::new`, de `validate_deep` y de los sitios
de log; un cambio y una prueba de regresión por hallazgo; verificación local
(`cargo fmt`, `clippy`, `cargo test -p amatl-core --lib`, `cargo package`, sonda
`bwrap`) más sondas de carga para los hallazgos C2a y C2b. Este documento es un
informe fechado, no una norma: los controles viven
en `docs/security/threat-model.md` y `docs/security/asvs-checklist.md`.

## Resumen

| ID | Hallazgo | Severidad | Estado |
|---|---|---|---|
| H1 | El proceso hijo hereda el entorno del servicio (`server.token_env`, `credential_env`) | Alta | Remediado |
| H2 | Una ruta relativa de extractor o harness se resuelve contra el directorio de trabajo | Media | Remediado |
| H3 | No existía una forma segura de instrumentar `HttpRequest`: derivar `Debug` publica URL y cabeceras | Media | Remediado |
| H4 | El harness re-heredaba dentro del sandbox lo que el padre no había limpiado | Media | Remediado |
| H5 | Riesgos aceptados sin declarar (`admin` equivale a ejecución; sondas públicas) | Baja | Documentado |
| C1 | `include_str!` del OpenAPI en un módulo `#[cfg(test)]`: rompe `cargo test` sobre el tarball, no la publicación | Baja (empaquetado) | Refutado como bloqueo de release y **reparado el 2026-10-27** (ver C1) |
| C2a | Test intermitente `extract::tests::fixed_cli_contract_…`: `ETXTBSY` al lanzar el script recién escrito | Baja | Remediado en esta revisión |
| C2b | Test intermitente `execution::tests::success_serializes_an_empty_error_list`: plazo de 1 000 ms con reserva de 750 ms | Baja | Remediado en esta revisión |
| C3 | Citas `file:line` desplazadas en `docs/security` por una inserción en `providers/http.rs` | Baja (integridad documental) | Remediado |
| C4 | El `readme` de `amatl-core` apunta fuera del paquete y Cargo lo reescribe: aviso de empaquetado y campo que no describe lo publicado | Baja (empaquetado) | Remediado en esta revisión |

## H1 — El hijo heredaba el entorno del servicio (Alta)

**Causa.** Ningún `Command::new` llamaba a `env_clear`. Por defecto el hijo hereda
el entorno del padre, que es donde viven `server.token_env` y el `credential_env`
de cada provider: un fallo de memoria en Trafilatura, `pdftotext` o Chromium
—procesos que parsean contenido hostil— se convertía en robo de credenciales, sin
señal visible en logs. En `extract.rs:43-150` se veía el `spawn` sin limpieza.

**Cambio.** `child_process::scrub_environment` (`crates/amatl-core/src/child_process.rs:36-39`)
vacía el entorno y reenvía solo una lista blanca de nombres que no pueden contener
secretos (`PATH`, `HOME`, `TMPDIR`, `LANG`, `LC_ALL`, `TZ`;
`crates/amatl-core/src/child_process.rs:33`); lo que un hijo necesite además se
nombra explícitamente con `forward_environment`
(`crates/amatl-core/src/child_process.rs:49-55`). Aplicado en los cuatro sitios de
spawn: `crates/amatl-core/src/extract.rs:70`, `crates/amatl-core/src/extract.rs:123`,
`crates/amatl-core/src/ingest.rs:327` y `crates/amatl-core/src/render.rs:207-216`
(el renderer reenvía solo `AMATL_CHROMIUM_BIN`, `AMATL_CHROMIUM_TASKS_MAX`,
`XDG_RUNTIME_DIR` y `DBUS_SESSION_BUS_ADDRESS`).

**Prueba.** Un canario por camino: `crates/amatl-core/src/child_process.rs:127`
(recorre la lista blanca y comprueba que el hijo no ve nada más), canarios sobre el
spawn real en `extract.rs` e `ingest.rs`, y
`crates/amatl-core/tests/deep_phase5.rs:663` sobre `ChromiumRenderer::render` con
un harness de reemplazo.

**Residual.** El hijo sigue teniendo el sistema de ficheros y la red de la cuenta
de servicio: no hay sandbox de SO, seccomp ni separación de uid
(`docs/security/threat-model.md:104`).

## H2 — Ruta relativa resuelta contra el directorio de trabajo (Media)

**Causa.** `deep.extractor.executable` y `deep.renderer.sandbox_path` se aceptaban
como cadena libre y se pasaban tal cual a `Command::new`. Una ruta relativa con
separador (`./trafilatura`, `../bin/x`) se resuelve contra el directorio de trabajo
del proceso, que el operador no fija en el arranque real (unidad systemd,
lanzamiento desde un directorio escribible): el servicio ejecutaba un binario
distinto del previsto sin que la configuración lo dijera.

**Cambio.** `validate_deep` (`crates/amatl-core/src/config/deep_config.rs:416`)
rechaza toda ruta relativa con separador en los dos campos
(`crates/amatl-core/src/config/deep_config.rs:477-483` para
`deep.extractor.executable` y `crates/amatl-core/src/config/deep_config.rs:484-491`
para `deep.renderer.sandbox_path`), con el predicado `is_relative_program_path`
(`crates/amatl-core/src/config/deep_config.rs:498-500`).
Se admite ruta absoluta o nombre simple resuelto por `PATH`.

**Prueba.** `crates/amatl-core/src/config/mod.rs:1235`
(`relative_deep_program_paths_are_rejected`). Documentado en
`docs/configuracion.md:292` y `docs/configuracion.md:306`.

**Residual.** Validar la *forma* de la ruta no valida el binario. Sin hash fijado,
un token `admin` que elige el ejecutable sigue equivaliendo a ejecución de código
como la cuenta de servicio (`docs/security/threat-model.md:89`).

## H3 — Redacción de logs incompleta (Media)

**Causa.** `HttpRequest` no implementaba `Debug`, así que el camino disponible para
instrumentarla era derivarlo —y una `Debug` derivada publica la URL completa (con
la `api_key` en la query de algunos providers) y el valor de `Authorization`. El
`url = %sanitized_url` del transporte
(`crates/amatl-core/src/providers/http.rs:140-148`) cubría solo ese sitio de log,
no el tipo: la redacción era una convención que cada futuro adaptador podía romper.

**Cambio.** `Debug` escrita a mano
(`crates/amatl-core/src/providers/http.rs:72-89`) que imprime la URL saneada, los
*nombres* de cabecera, el timeout y el tamaño del cuerpo. El sitio de log natural
(`?request`) queda redactado por defecto.

**Prueba.** `crates/amatl-core/src/providers/http.rs:223` comprueba que el valor del
secreto no aparece en la representación.

**Residual.** La redacción vive en el tipo, no en un envoltorio obligatorio: un
adaptador que guarde su propia copia del `Url` crudo puede seguir logueándolo
(`docs/security/threat-model.md:67`).

## H4 — El harness re-heredaba dentro del sandbox (Media)

**Causa.** El sandbox de Chromium creaba namespaces, pero el navegador arrancaba
con el entorno que le pasara `bwrap`; sin `--clearenv`, cualquier variable presente
en el entorno del proceso de servicio llegaba al navegador, la pieza con más
superficie de las tres.

**Cambio.** `--clearenv` en el harness (`packaging/amatl-chromium-sandbox:73`),
antes de sus propios `--setenv`.

**Prueba.** `bwrap --clearenv` verificado sobre el harness real: el canario del
padre no aparece y un `--setenv HOME=…` explícito sobrevive, es decir que se limpia
lo heredado y solo eso. En CI, un paso que sustituye Chromium por un *stand-in* que
vuelca `env` al DOM (`.github/workflows/chromium-isolation.yml:99`), más el render
real de Chromium sin acceso a red (`docs/security/chromium-isolation.md:15`,
`docs/security/chromium-isolation.md:41`).

## H5 — Riesgos aceptados, ahora declarados (Baja)

Dos riesgos no eran defectos sino decisiones implícitas, y una checklist que los
calla se lee como cobertura. Se documentan como aceptados en
`docs/security/threat-model.md:89` (`admin` equivale a ejecución de código como la
cuenta de servicio: `PATCH /deep/extractor` y `PATCH /deep/renderer` eligen el
programa) y `docs/security/threat-model.md:95` (`/health`, `/ready` y `/metrics`
son públicos en la interfaz que el operador publique, y `/metrics` expone
contadores, latencias y agregados). El disparador de revisión del modelo incorpora
ya el cambio de entorno de un proceso externo
(`docs/security/threat-model.md:110`).

## C1 — Refutado como bloqueo de release y reparado el 2026-10-27: el `include_str!` del OpenAPI no impide publicar (Baja, empaquetado)

**Afirmación de la primera versión, refutada.** Se sostuvo que
`crates/amatl-server/src/tests.rs:5084` —y también `:5117`, `:5206` y `:5357`— usa
`include_str!("../../../docs/api/openapi.yaml")`, que Cargo no empaqueta el fichero y
que por eso `cargo publish -p amatl-server` falla hoy, como hallazgo de severidad
Alta de release.

**Por qué era falso.** Las cuatro citas existen, pero `tests.rs` es un módulo de
pruebas: `crates/amatl-server/src/lib.rs:3368-3369` lo declara con
`#[cfg(test)] mod tests;`. El `include_str!` vive en código que **no se compila** al
verificar el paquete, porque esa verificación construye la librería y los binarios,
no los objetivos de prueba. La evidencia citada (`cargo package -p amatl-server --list
--allow-dirty --locked --no-verify`) no compila nada: `--list` solo enumera el
contenido del tarball y `--no-verify` desactiva justamente la construcción. De ahí se
sigue que el fichero no viaja en el tarball, no que la verificación falle. Tampoco
existe una bandera `--verify` que activar: `cargo 1.97.1` verifica por defecto y
`--no-verify` es la única bandera relacionada.

**Comprobación.** Sonda mínima autocontenida, no incluida en este repositorio: un crate
aparte con un `include_str!` de un fichero ausente y fuera del crate, dentro de un
módulo `#[cfg(test)]`.

| Comando | Resultado |
|---|---|
| `cargo package --allow-dirty` | exit 0: `Verifying pkgprobe` → `Compiling pkgprobe` → `Finished dev profile` |
| `cargo test --no-run` | exit 101: `error: couldn't read src/../../missing-file.txt: No such file or directory` |

El patrón rompe `cargo test`, no `cargo package` (ni, por extensión, `cargo publish`).
Es reproducible en el propio repositorio: `cargo package -p amatl-core --allow-dirty
--locked` termina en `Finished dev profile [unoptimized + debuginfo] target(s) in
2.74s` tras empaquetar 102 ficheros.

**Consecuencia real (Baja, deuda de empaquetado).** El tarball de `amatl-server`
incluía `src/tests.rs` pero no el contrato OpenAPI (13 ficheros, ninguno con
`openapi` en el nombre), así que **desempaquetar el tarball y ejecutar `cargo test` no
compilaba**. Afectaba a quien pruebe el código empaquetado o a un árbol sin `docs/`,
no a la publicación; en un clon del repositorio el contrato estaba presente (85 113
bytes, no ignorado por git) y la suite siempre compiló. Reparado el 2026-10-27 moviendo
el contrato al crate; ver la nota al final de esta sección.

**Precisión del 2026-10-27: el bloqueo del tarball es doble y hoy solo se observa el
primero.** Sobre el tarball extraído —el que deja `cargo package` en
`target/package/amatl-server-0.1.0-rc.1/`—, `cargo test --offline --no-run` muere en
la resolución, antes de compilar nada: `no matching package named 'amatl-core'
found`. El manifiesto empaquetado ya reescribió la dependencia a
`version = "0.1.0-rc.1"` sin `path`, y esa versión no está publicada, así que hoy el
`include_str!` no es lo primero que rompe: es lo que romperá después, cuando
`amatl-core` y `amatl-ui` estén publicadas. Que romperá es un hecho estático,
comprobable sin resolver dependencias: en el tarball, `src/tests.rs` sigue llevando
`include_str!("../../../docs/api/openapi.yaml")`, `docs/` no existe dentro del
paquete y la ruta resuelve fuera de él. Y la razón de que la verificación de
`cargo package` no lo detecte es la de arriba —verifica construyendo la librería, y
`src/tests.rs` cuelga de un `#[cfg(test)]`—, no una bandera ausente.

**La otra mitad de la afirmación también era falsa.** Se añadía que «el mismo patrón
ya se aplicó en `amatl-core` (lectura en tiempo de ejecución con `read_to_string` más
una guarda de presencia)». Fuera de `crates/amatl-server/src/tests.rs` no hay ninguna
referencia a `openapi` en código Rust, y en `amatl-core` no existe esa lectura en
tiempo de ejecución: no hay precedente que copiar y la decisión sigue abierta.

**Por qué `cargo publish -p amatl-server` sí falla hoy (causa distinta).**
`crates/amatl-server/Cargo.toml:16` declara
`amatl-core = { version = "0.1.0-rc.1", path = "../amatl-core" }` (y
`crates/amatl-server/Cargo.toml:17` hace lo propio con `amatl-ui`). Al empaquetar,
Cargo elimina el `path` y la verificación resuelve la dependencia contra el índice de
crates.io, donde `amatl-core 0.1.0-rc.1` no está publicado. Reproducido con
`cargo package -p amatl-server --allow-dirty --locked`, esta vez **sin** `--no-verify`,
es decir con la verificación activa:

```
error: failed to prepare local package for uploading

Caused by:
  no matching package named `amatl-core` found
  location searched: crates.io index
  required by package `amatl-server v0.1.0-rc.1 (/home/panda/amatl/crates/amatl-server)`
```

El fallo ocurre al preparar el paquete, antes de compilar nada, y por eso es
independiente del `include_str!`: el resultado es el mismo con y sin él. Es una
condición de secuencia de release —publicar `amatl-core` y `amatl-ui` antes que
`amatl-server`—, no un defecto del código, y queda como decisión de publicación, no
como hallazgo de seguridad. Si se quisiera que el tarball se pudiera probar sin
`docs/`, la opción es leer el OpenAPI en tiempo de ejecución con `read_to_string` más
una guarda de presencia; no se ha aplicado.

**La corrección aparente tampoco sirve, y falla en silencio.** La tentación es
declarar el fichero en el `include` del crate. Se probó el 2026-10-27 sobre
`167fb8dd` añadiendo `"../../docs/api/openapi.yaml"` al `include` de
`crates/amatl-server/Cargo.toml`: `cargo package -p amatl-server --list --locked
--allow-dirty` termina con exit 0, **no emite ningún aviso** y el listado sigue
teniendo los mismos 13 ficheros, sin el OpenAPI. Cargo descarta en silencio las
entradas que salen del directorio del paquete, así que un `include` con `..` deja
el manifiesto peor que antes —promete un fichero que no viaja— y sin señal alguna
de que no se cumplió. La sonda se revirtió y el árbol quedó como estaba. Quedan,
por tanto, dos salidas reales y ninguna es gratis: duplicar el contrato dentro del
crate (con riesgo de deriva) o mover el contrato canónico al crate. La decisión se
tomó al día siguiente; ver la nota siguiente.

**Reparado el 2026-10-27: el contrato se movió al crate.** De las dos salidas abiertas
se eligió la segunda, mover el canónico en lugar de duplicarlo: el contrato pasó de
`docs/api/` a la raíz del crate con `git mv` (85 113 bytes, sin un solo cambio de
contenido), y los cuatro `include_str!` de `crates/amatl-server/src/tests.rs` —más los
tres textos que citaban la ruta— apuntan ahora a `crates/amatl-server/openapi.yaml`. El contrato sigue
siendo uno solo y vive junto al código que lo sirve; `README.md`, `docs/README.md` y
los dos documentos de `docs/experiments/` apuntan a la ruta nueva, y `docs/api/`
conserva `mcp.md`.

Evidencia (toda local, sobre el árbol con el cambio):

| Comprobación | Comando | Resultado |
|---|---|---|
| El contrato viaja en el paquete | `cargo package -p amatl-server --list --locked --allow-dirty --no-verify` | 14 ficheros, `openapi.yaml` en la raíz del paquete (antes 13, ninguno con `openapi`) |
| Los tests de contrato siguen pasando | `cargo test --locked -p amatl-server --lib openapi` | 5 passed; 0 failed |
| El tarball compila y ejecuta esos tests | tarball real extraído, `cargo test --offline --lib openapi` | `5 passed; 0 failed` dentro del tarball |
| El fallo, reproducido sobre el tarball anterior | mismo tarball, anterior al arreglo | 4 errores `couldn't read` del contrato → `could not compile amatl-server (lib test)` |

La última fila es la reproducción directa del hallazgo y la primera su cierre: el
tarball anterior moría compilando `src/tests.rs`, uno por cada `include_str!`, y el
actual no. Los dos `cargo package` se obtuvieron con `--no-verify` porque la
preparación del paquete falla antes, por la dependencia `path` sin publicar; para
compilar los tests dentro del tarball se aportó esa resolución a mano con un `[patch]`
hacia los crates locales, que es lo que aísla el `include_str!` del problema de orden
de release. Ese problema **no** cambia con este arreglo: `cargo package -p
amatl-server` sin `--no-verify` sigue fallando al preparar el paquete mientras
`amatl-core` y `amatl-ui` no estén publicadas.

## C2a — Test intermitente por `ETXTBSY` al lanzar el extractor de prueba (Baja, remediado en esta revisión)

**Causa.** `crates/amatl-core/src/extract.rs:650`
(`extractor.extract(…).unwrap()`, dentro de
`fixed_cli_contract_requests_metadata_without_network_arguments`, que empieza en
`crates/amatl-core/src/extract.rs:621`) falla con
`called Result::unwrap() on an Err value: Failed`. Ese `Failed` procede del mapeo del
error de `Command::spawn` (`crates/amatl-core/src/extract.rs:130-137`): solo
`ErrorKind::NotFound` se traduce a `Unavailable`; cualquier otro error de `spawn` cae
en `ExtractError::Failed`.

El error real es `ETXTBSY` (`ExecutableFileBusy`, os error 26), capturado en esta
revisión con instrumentación temporal sobre el `spawn` de `extract`:

```
AMATL_DIAG spawn kind=ExecutableFileBusy error=Text file busy (os error 26)
```

**Mecanismo.** El test escribe un script (`std::fs::write`,
`crates/amatl-core/src/extract.rs:630`), le da permiso de ejecución y lo lanza en el
acto. El `fd` de escritura sigue abierto mientras otro hilo del mismo proceso llama a
`fork()`; el hijo hereda ese descriptor y `execve` comprueba `deny_write_access`
**antes** de que `O_CLOEXEC` cierre la copia heredada. Si el `exec` del extractor cae
en esa ventana, el kernel devuelve `ETXTBSY`. No es una carrera de puerto ni de nombre
de fichero temporal: es la interacción entre una escritura reciente y un `fork`
concurrente.

**Evidencia (capturada antes del remedio).**
- Sonda autocontenida que reproduce el patrón (`write` + `chmod` + `spawn` en hilos
  concurrentes, fuera de este repositorio): **860 fallos de 19 200 intentos** (4,5 %),
  todos `os error 26`, con rutas distintas por intento.
- `cargo test --workspace` ×12: falla 1 vez (run 3), con exactamente este test
  (`error: test failed, to rerun pass -p amatl-core --lib`); los otros 11 runs, 0
  fallos.
- 24 ejecuciones de la suite con 8 binarios concurrentes (`--test-threads 16`,
  binario instrumentado): 1 fallo de este test, y la línea `AMATL_DIAG spawn
  kind=ExecutableFileBusy` como única anomalía registrada.

**Alcance.** Es un artefacto de la prueba: el código de producción nunca escribe el
programa que va a lanzar. El riesgo de producción equivalente es estrecho —un operador
que escriba un script y lo configure con `PATCH /deep/extractor` en el mismo
instante—, pero el `Failed` genérico hace que el síntoma sea indistinguible de un fallo
real del extractor, y eso sí tiene coste de diagnóstico. El reintento cierra esa ventana
en ambos casos; un `spawn` que falle por otra causa sigue reportándose como `Failed`.

**Remedio aplicado.** `child_process::spawn_with_busy_retry`
(`crates/amatl-core/src/child_process.rs:68-103`) reintenta el `spawn` solo cuando el
error es `ErrorKind::ExecutableFileBusy`: siete intentos con retroceso exponencial desde
1 ms (`BUSY_SPAWN_RETRIES` y `BUSY_SPAWN_FIRST_DELAY`,
`crates/amatl-core/src/child_process.rs:61-66`), 127 ms en el peor caso, muy por debajo
del plazo más corto de cualquier llamador. `Command::spawn` toma `&mut self`, así que se
reintenta el mismo `Command` —mismos argumentos, mismo `stdio`, mismo entorno ya
depurado—. Está aplicado en los tres sitios que lanzan un programa de ruta configurable:
`crates/amatl-core/src/extract.rs:70`, `crates/amatl-core/src/extract.rs:123` y
`crates/amatl-core/src/ingest.rs:327`. Se eligió esta opción y no la de convertir el
script en *fixture* porque la primera cubre también el caso de producción —un operador
que escribe un script y lo configura con `PATCH /deep/extractor` en el mismo
instante— y porque deja el test intacto.

**Evidencia del remedio (sonda aislada).** Una sonda temporal —un test de integración
en `crates/amatl-core/tests/`, escrito para esta medición y borrado al cerrar el
hallazgo, que reproduce el patrón `write` + `chmod` + `spawn` con rutas nuevas por
intento— midió en 24 núcleos con 24 procesos de carga:

| Control | Concurrencia | `spawn` | Intentos | Fallos `ETXTBSY` |
|---|---|---|---|---|
| A | 1 hilo, aislado | crudo | 4 000 | 0 |
| B | 16 hilos × 250 | crudo | 4 000 | 54 (1,35 %) |
| C | 16 hilos × 250 | `spawn_with_busy_retry` (el de producción) | 4 000 | 0 |

B demuestra que la carrera es real cuando el mismo proceso escribe y lanza desde varios
hilos; C, que el helper de producción la elimina sin alterar el resultado del `spawn`. El
control A solo es válido si ningún otro hilo del proceso está en `fork`: un `fork` desde
cualquier hilo duplica *todos* los descriptores abiertos, y por eso A registró 4 fallos
cuando sus tests hermanos corrían en paralelo dentro del mismo proceso —observación que
confirma el mecanismo descrito arriba—. El control A se midió con
`--test-threads=1 --nocapture` (con la salida capturada no se ven los contadores), que es
la condición que hace válida esa fila.

## C2b — Test intermitente por presupuesto de plazo demasiado ajustado (Baja, remediado en esta revisión)

**Causa.** `execution::tests::success_serializes_an_empty_error_list`
(`crates/amatl-core/src/execution.rs:1295`) espera `SearchStatus::Success`
(`crates/amatl-core/src/execution.rs:1303`) y recibe `PartialSuccess` cuando la
máquina está saturada.

**Mecanismo.** El test usa `Budget::new(1, 1_000)`
(`crates/amatl-core/src/budget.rs:180`): 1 000 ms de plazo total. En cada ronda el
orquestador calcula `remaining_ms = deadline_ms - elapsed_ms`
(`crates/amatl-core/src/execution.rs:341-342`) y lo pasa a la decisión de ronda
(`crates/amatl-core/src/execution.rs:352`), que devuelve `SearchStopReason::DeadlineNear`
en cuanto `remaining_ms < policy.minimum_remaining_deadline_ms`
(`crates/amatl-core/src/execution.rs:944-945`), con el valor por defecto de **750 ms**
(`crates/amatl-core/src/progressive.rs:44`, campo declarado en
`crates/amatl-core/src/progressive.rs:22`). Con 1 000 ms de presupuesto
basta con que la primera ronda tarde **≥ 250 ms** de reloj de pared para que la ronda
se marque `DeadlineNear`, el orquestador añada una degradación
(`crates/amatl-core/src/execution.rs:454-461`) y el estado final sea `PartialSuccess`
en vez de `Success`. No hay estado global ni carrera: es tiempo de reloj de pared
frente a un umbral que el test deja a 250 ms de margen.

**Evidencia (capturada antes del remedio).**
- 320 ejecuciones del test con 150 procesos de carga en 24 núcleos: 25+ decisiones
  `DeadlineNear` con `elapsed_ms` entre 266 y 338 ms
  (`AMATL_DIAG2 stop=deadline_near elapsed_ms=… remaining_ms=… results=1`).
- 24 ejecuciones de la suite con 8 binarios concurrentes: 3 fallos de este test, cada
  uno acompañado de su línea `AMATL_DIAG2 stop=deadline_near` (`elapsed_ms` 277, 326 y
  430 ms → `remaining_ms` 723, 674 y 570 ms), y ningún otro test de `execution::`
  afectado.

**Alcance.** Solo prueba: en producción el plazo lo fija el operador y un
`PartialSuccess` con la respuesta completa y una degradación de plazo es el
comportamiento diseñado. El defecto es que el test fija un presupuesto (1 000 ms)
incompatible con su propio umbral de degradación (750 ms) bajo carga.

**Remedio aplicado.** Los sitios cuyo aserto depende del plazo usan ahora el mismo
presupuesto que el servicio aplica de verdad: `Budget::new(…, 1_000)` →
`Budget::new(…, 8_000)`, que es el valor por defecto de `timeouts.global_ms`
(`crates/amatl-core/src/config/execution.rs:34`). Con 8 000 ms la reserva de 750 ms
(`crates/amatl-core/src/progressive.rs:44`) pasa del 75 % del presupuesto al 9,4 %, y la
primera ronda tendría que tardar **≥ 7 250 ms** de reloj de pared —más de veinte veces el
peor caso medido, 430 ms— para provocar `DeadlineNear`. El segundo argumento de
`Budget::new` (llamadas a proveedor) no cambia en ningún sitio, así que no se añaden
llamadas ni se relaja ningún aserto: solo se deja de exigir `Success` con 250 ms de
margen. Sitios: `crates/amatl-core/src/execution.rs:1291`,
`crates/amatl-core/src/execution.rs:1310` y `crates/amatl-core/src/execution.rs:1330`;
`crates/amatl-core/src/operational.rs:121` y `crates/amatl-core/src/operational.rs:174`;
`crates/amatl-core/tests/providers_phase1.rs:90`;
`crates/amatl-core/tests/persistence_phase3.rs:67` y
`crates/amatl-core/tests/persistence_phase3.rs:116`, y
`crates/amatl-core/tests/result_pipeline_phase2.rs:68`. Se descartó la alternativa de
bajar `minimum_remaining_deadline_ms` con `with_search_policy`
(`crates/amatl-core/src/execution.rs:157`): tocaría la política de producción para
arreglar una prueba.

**Por qué no se tocan los `Budget::new(…, 1_000)` restantes.** Quedan doce y ninguno
depende del plazo. Seis construyen solo el plan, y `build_search_plan` no consulta el
plazo —el único punto que lo lee es `crates/amatl-core/src/execution.rs:279-297`, dentro
de la ejecución—: `crates/amatl-core/src/operational.rs:296`,
`crates/amatl-core/src/gaps.rs:471`, `crates/amatl-core/src/cache.rs:299`,
`crates/amatl-core/tests/deep_phase5.rs:172`,
`crates/amatl-core/src/providers/marginalia.rs:341` y
`crates/amatl-core/src/providers/searxng.rs:391`. Los otros seis sí ejecutan el
orquestador, pero su aserto es invariante ante la degradación:
`crates/amatl-core/tests/providers_phase1.rs:64` y
`crates/amatl-core/tests/result_pipeline_phase2.rs:30` esperan `PartialSuccess`, que es
justo lo que produce una degradación; `crates/amatl-core/tests/providers_phase1.rs:110` y
`crates/amatl-core/tests/result_pipeline_phase2.rs:109` esperan `Failure` con cero
resultados, y una degradación no puede convertir eso en `Success` ni en `PartialSuccess`
—ambos exigen resultados—; `crates/amatl-core/tests/persistence_phase3.rs:99` y
`crates/amatl-core/tests/result_pipeline_phase2.rs:92` no afirman estado alguno (cuentan
intentos del proveedor y miran rangos). En los seis el transporte es un
`MockProvider`/`FixtureTransport` sin retardo, así que `remaining_ms` se queda pegado a
~1 000 ms y no cruza los 750 ms.

**Evidencia del remedio (suite completa bajo carga).** 12 ejecuciones de
`cargo test -p amatl-core -- --test-threads=16` con 24 procesos `yes` ocupando los 24
núcleos (la carga se comprobó con `pgrep -c -x yes` = 24 durante toda la tanda): **12 de
12 con `cargo_exit=0`**, 5 976 pruebas superadas y 0 fallos repartidas en 216 ejecuciones
de binario de prueba, sin una sola línea `FAILED`, `panicked`, `ETXTBSY` ni
`DeadlineNear`. En la ronda de diagnóstico —24 ejecuciones con 8 binarios concurrentes— el
mismo test falló 3 veces con presupuesto de 1 000 ms; con 8 000 ms no falla ni en la tanda
más exigente (16 hilos en vez de 8). La sonda de C2a se excluyó de estas tandas
(`--skip probe_*`) porque generaba miles de `spawn` adicionales; su evidencia es la de su
propia tabla.

## C3 — Citas `file:line` desplazadas (Baja, remediado en esta revisión)

Insertar el `Debug` redactado en `crates/amatl-core/src/providers/http.rs` (29
líneas) desplazó cuatro citas de `docs/security` que apuntaban a código de
producción: la del error genérico del transporte (`providers/http.rs:109-119`, que
era el bloque `map_err` con `url = %sanitized_url`, hoy en
`crates/amatl-core/src/providers/http.rs:140-148`) y la del propio transporte
(`64-101`). El gate `tools/check_doc_refs.py` no puede detectarlas: siguen
resolviendo a líneas existentes, solo dejaron de describir lo que citan. Re-ancladas
en `docs/security/threat-model.md:67`, `docs/security/threat-model.md:68`,
`docs/security/threat-model.md:83` y `docs/security/asvs-checklist.md:45`. Se
comprobó además que las citas a `extract.rs`, `ingest.rs`, `config/mod.rs` y
`config/deep_config.rs` siguen cayendo en el mismo bloque tras los cambios
(desplazamientos de hasta 3 líneas dentro de la misma región).

## C4 — El `readme` de `amatl-core` no describía lo que se publica (Baja, remediado en esta revisión)

**Causa.** `crates/amatl-core/Cargo.toml:10` declaraba
`readme = "../../README.md"`, una ruta fuera del paquete, mientras el crate ya
tiene su propio `crates/amatl-core/README.md` (3 999 bytes). Cargo resuelve esa
colisión a favor del fichero local y **reescribe** el campo en el manifiesto
publicado, así que la declaración de origen no describía el paquete que se
publica, y cada `cargo package -p amatl-core` —incluido el `--list` que ejecuta
`.github/workflows/publish-crates.yml`— imprimía este aviso:

```
warning: readme `../../README.md` appears to be a path outside of the package, but there is already a file named `README.md` in the root of the package. The archived crate will contain the copy in the root of the package. Update the readme to point to the path relative to the root of the package to remove this warning.
```

**Alcance.** El artefacto publicado no estaba mal: el tarball
`amatl-core-0.1.0-rc.1.crate` lleva el README del crate y su `Cargo.toml` ya dice
`readme = "README.md"`. El defecto es del manifiesto de origen —un campo muerto
que no describe la realidad— más el ruido de empaquetado. No afecta a los otros
tres crates: `amatl-cli`, `amatl-server` y `amatl-ui` usan la misma ruta,
`readme = "../../README.md"`, sin aviso, porque no tienen README propio y Cargo
copia el de la raíz al paquete; comprobado en el tarball de `amatl-server`, que
lleva el README raíz (`# AMATL`) y `readme = "README.md"` en su manifiesto.

**Cambio.** `readme = "README.md"` en `crates/amatl-core/Cargo.toml:10`, que es lo
que Cargo ya publicaba. El contenido del tarball es el mismo; lo que cambia es que
el manifiesto dice la verdad y el aviso desaparece.

**Prueba.** `cargo package -p amatl-core --list --locked --allow-dirty`: antes del
cambio, 1 aviso por stderr; después, exit 0 con stderr vacío (0 bytes) y los
mismos 102 ficheros. `cargo package --workspace --allow-dirty --locked` sigue
terminando en exit 0 sin ningún aviso.

## Verificación ejecutada

| Comprobación | Comando | Resultado |
|---|---|---|
| Formato | `cargo fmt --check` | limpio |
| Lints | `cargo clippy --workspace --all-targets` | 0 avisos |
| Pruebas del núcleo | `cargo test -p amatl-core --lib` | 395 passed; 0 failed |
| Empaquetado del núcleo | `cargo package -p amatl-core --allow-dirty --locked` | 102 ficheros, 3.2 MiB; verificación compila y termina |
| Contenido del tarball del servidor | `cargo package -p amatl-server --list --allow-dirty --locked --no-verify` | 13 ficheros, 0 con `openapi`; incluye `src/tests.rs` |
| Empaquetado del servidor | `cargo package -p amatl-server --allow-dirty --locked` | falla al preparar, antes de compilar: «no matching package named amatl-core found» (dependencia `path`, no el `include_str!`) |
| `include_str!` roto en `#[cfg(test)]` | sonda mínima: `cargo package --allow-dirty` y `cargo test --no-run` | 0 y 101: empaqueta y verifica, pero las pruebas no compilan |
| Citas de documentación | `python3 tools/check_doc_refs.py` | 12 known-dead, 0 new (exit 0) |
| `ETXTBSY` en `spawn` | sonda mínima: `write` + `chmod` + `spawn` concurrentes | 860 fallos / 19 200 intentos (4,5 %), todos os error 26 |
| Suite completa bajo carga | `cargo test --workspace` ×12 | 1 fallo (run 3): `extract::tests::fixed_cli_contract_…`, con `AMATL_DIAG spawn kind=ExecutableFileBusy` |
| Suite completa bajo carga, 8 binarios | 24 ejecuciones, `--test-threads 16` | 4 fallos: 1 de `extract::…` y 3 de `execution::success_serializes_an_empty_error_list`, cada uno con su `AMATL_DIAG2 stop=deadline_near` |
| Los dos tests intermitentes, aislados | `cargo test -p amatl-core --lib -- extract::tests` y `-- execution::tests` | pasan; sin carga no se reproducen |
| `ETXTBSY` en `spawn`, aislado y bajo carga | sonda temporal (borrada al cerrar el hallazgo): 1 hilo / 16 hilos crudos / 16 hilos con `spawn_with_busy_retry` | 0 / 54 / 0 fallos de 4 000 intentos (1,35 % solo en el control crudo concurrente) |
| Suite completa bajo carga, tras los remedios | `cargo test -p amatl-core -- --test-threads=16` ×12, 24 procesos `yes` en 24 núcleos | 12/12 `cargo_exit=0`; 5 976 pruebas, 0 fallos; sin `FAILED` ni `DeadlineNear` |
| `--clearenv` sobre el harness real | sonda `bwrap` + render Chromium | canario ausente, `HOME` explícito presente, render `rendered` |
| Aviso de `readme` en el núcleo | `cargo package -p amatl-core --list --locked --allow-dirty` | antes: 1 aviso (ruta `..` con `README.md` local); tras el arreglo: stderr vacío, 102 ficheros, exit 0 |
| Empaquetado del workspace completo | `cargo package --workspace --allow-dirty --locked` | exit 0: los cuatro tarballs verifican compilando, `amatl-server` incluido |
| Entrada `..` en `[package] include` | sonda sobre `crates/amatl-server/Cargo.toml` (revertida) | exit 0, 0 avisos y el OpenAPI **no** entra en el tarball: 13 ficheros |

## Lo que no se puede afirmar

- No se ejecutó CI remoto. Los workflows `ci.yml`, `static-analysis`,
  `codeql-invariants` y `chromium-isolation` no se han disparado sobre `ce172d9d`, y
  una ejecución local de `cargo test` no sustituye macOS/Windows, MSRV, WebDriver ni
  CodeQL.
- No se ejecutó `cargo publish` ni se creó ninguna etiqueta; lo reproducido es
  `cargo package`, que es la fase que falla por la dependencia `path` sin publicar.
- Las pruebas de H1 fuera de `amatl-core` requieren `AMATL_CHROMIUM_INTEGRATION=1` y
  el harness instalado; sin esa variable se omiten en vez de fallar, así que un
  entorno sin `bwrap` no aporta evidencia alguna.
- C1 quedaba sin remediar en su parte real cuando se escribió esta revisión: el
  `include_str!` del OpenAPI no resolvía al ejecutar `cargo test` sobre el tarball de
  `amatl-server`. Se reparó el 2026-10-27 moviendo el contrato al crate (ver la nota
  al final de C1). Lo que nunca bloqueó es la publicación por ese motivo. Medido el
  2026-10-27 sobre el tarball anterior al arreglo: ese `cargo test` fallaba primero
  por la resolución de `amatl-core` —no publicada, sin `path` en el manifiesto
  empaquetado—, y el `include_str!` era el bloqueo siguiente, el que queda cuando las
  dependencias internas ya están publicadas: aportada esa resolución con un `[patch]`,
  la compilación moría con cuatro errores `couldn't read` del contrato.
- La deuda de dependencias que la primera versión daba por abierta (`chacha20`
  yanked, transitiva por `rand` y `rmcp`) **no estaba abierta en la base de este
  informe**: `git show ce172d9d:Cargo.lock` ya trae `chacha20 0.10.2`, y el cierre
  está documentado en `docs/security/supply-chain.md:111-115`. Enunciado corregido
  en «Correcciones».

## Correcciones a la primera versión de este informe

Esta revisión volvió a comprobar, una por una, las afirmaciones de la primera versión
contra el repositorio y contra sondas reproducibles. Dos hallazgos estaban mal:

- **C1 estaba mal diagnosticado.** Se basaba en una bandera `--verify` que no existe y
  en una lectura de `--list --no-verify` como si fuera la verificación del paquete. El
  `include_str!` del OpenAPI está en `#[cfg(test)]` y no impide empaquetar ni publicar;
  lo que rompe es `cargo test` sobre el tarball. El fallo real de publicación es la
  dependencia `path` de `amatl-core`/`amatl-ui`, una condición de orden de release. La
  afirmación de que `amatl-core` ya usaba lectura en tiempo de ejecución tampoco era
  cierta. Severidad corregida de Alta a Baja.
- **C2 estaba mal clasificado.** No era una carrera de puerto efímero en
  `fetch::transport_error_classification_tests`: son dos intermitencias distintas, en
  `extract::tests::fixed_cli_contract_…` (ETXTBSY en el `spawn` de un script recién
  escrito) y en `execution::tests::success_serializes_an_empty_error_list`
  (presupuesto de 1 000 ms con reserva de 750 ms). Se separan como C2a y C2b, con el
  mecanismo y la evidencia de cada una. El test de `fetch` pasa y no es intermitente
  en las mediciones de esta revisión.
- **H2 tenía dos citas desplazadas** (`config/deep_config.rs:467` y `:495`, esta
  última en una línea vacía). Re-ancladas a los bloques reales (`:477-483`, `:484-491`
  y `:498-500`).
- El resto de afirmaciones (H1, H3, H4, H5, C3 y el empaquetado de `amatl-core`) se
  comprobaron cita a cita contra el árbol actual y se mantienen.

**Añadido en la revisión de empaquetado del 2026-10-27.** Una tercera pasada,
centrada en el empaquetado y hecha sobre los tarballs ya extraídos, añade un
hallazgo y corrige un enunciado:

- **C4 (nuevo).** El aviso de `readme` de `amatl-core` lo emitía el mismo comando
  que la tabla de arriba registra como «verificación compila y termina»
  (`cargo package -p amatl-core --allow-dirty --locked`) y no se anotó: el aviso
  viaja por stderr, no por la salida que esa tabla resume. Remediado; ver C4.
- **`chacha20` (corrección).** El informe daba la deuda por abierta cuando su
  propia base ya la tenía cerrada: `git show ce172d9d:Cargo.lock` trae
  `chacha20 0.10.2`, y `docs/security/supply-chain.md:111-115` documenta el
  `cargo update -p chacha20` del 2026-10-26. Corregido también en «Lo que no se
  puede afirmar».
- **C1, confirmado con una prueba más fuerte.** La refutación no depende de la
  sonda mínima: `cargo package --workspace --allow-dirty --locked` verifica los
  cuatro tarballs compilándolos —`amatl-server` incluido— y termina en exit 0, con
  el único aviso de stderr siendo el `readme` de C4. Lo que sigue roto es `cargo
  test` sobre el tarball extraído, y la vía `[package] include` con `..` se probó y
  no sirve (ver C1).
