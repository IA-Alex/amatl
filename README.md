# AMATL

AMATL es un buscador generalista multi-fuente, Linux-first, rápido, modular y
resistente a fallos. Recibe una consulta, elige fuentes dentro de un presupuesto
global, normaliza, canonicaliza, deduplica y ordena resultados. Su flujo visible
es `buscar → revisar → abrir`.

No es un chatbot, generador de texto, crawler masivo, dashboard analítico, agente
autónomo ni sistema dependiente de LLM o de un único provider. Search permanece
ligero; Deep, Trafilatura, Chromium, SQLite y cachés son opcionales. El harness
de aislamiento de Chromium está verificado por separado; el backend permanece
desactivado hasta conectarlo sin transferir a Chromium el ownership de red.

## Invariantes visibles

- Un solo core sirve a CLI, UI, API y MCP.
- Un fallo parcial conserva resultados útiles y se identifica como
  `partial_success`.
- El orquestador es el único dueño del Budget y deadline globales.
- Search no descarga páginas ni expone `final_url`; Deep es la única frontera de
  fetch de red. La ingestión de archivos permanece local y separada.
- La política `data_policy` gobierna toda salida de red. El perfil `isolated`
  bloquea providers, Deep fetch, MCP fetch, canarios e inferencia remota antes
  de conectar; no elimina los motores deterministas de extracción/evidencia ni
  hace obligatorio un LLM.
- Los secretos se leen de variables de entorno, nunca de `amatl.toml`.
- Sin SQLite o caché, Search conserva su comportamiento correcto.
- Ningún provider real está activo por defecto ni puede omitir su revisión de
  términos, coste y operación.

## Estado e instalación

El workspace declara la candidata `0.1.0-rc.1`. El pipeline reproducible de
release construye y verifica en GitHub Actions un binario Linux musl, cuatro
SBOM CycloneDX, paquetes `.deb`, `.rpm` y Arch, el archivo reproducible y sus
checksums SHA-256. Una candidata es oficial únicamente cuando aparece bajo un
tag anotado coincidente en [GitHub Releases](https://github.com/IA-Alex/amatl/releases).
La instalación desde un checkout permanece disponible:

```bash
cargo install --locked --path crates/amatl-cli
```

La distribución objetivo es:

| Vía | Prioridad | Estado actual |
|---|---|---|
| Binario Linux musl precompilado | Principal | Se publica en la RC etiquetada junto con checksum y SBOM |
| `cargo install` desde fuente | Alternativa | Disponible desde checkout con `--path`; crates.io no verificado/publicado |
| `.deb` / `.rpm` / Arch | Integración nativa | El workflow RC produce paquetes verificables; `packaging/PKGBUILD` permite revisión/publicación posterior en AUR |

No uses un enlace o paquete de terceros como release oficial. Desarrollo y
build actual requieren Rust 1.88 o posterior por el grafo bloqueado; CI usa
`stable`. Consulta [DEVELOPMENT.md](DEVELOPMENT.md).

## Inicio rápido

```bash
cp amatl.example.toml amatl.toml
cargo run -p amatl-cli -- search "rust async" --json --mock
```

`--mock` es una ayuda local determinista; no consulta Internet. Para providers
reales hay que completar primero la [gobernanza](docs/gobernanza-providers.md),
habilitarlos en la configuración y exportar la credencial correspondiente.

Para un ejercicio con datos sensibles, usa en tu `amatl.toml`:

```toml
[data_policy]
profile = "isolated"
egress = "deny"
inference = "local_only" # o "disabled"
```

`amatl config` y `amatl doctor` muestran la política efectiva. Esta barrera es
del proceso AMATL; añade firewall/sandbox del host y usa sólo clientes locales
si necesitas aislamiento verificable de todo el entorno.

Servidor UI/API/MCP en loopback:

```bash
export AMATL_SERVER_TOKEN="$(openssl rand -hex 32)"
cargo run -p amatl-cli -- serve --mock
```

Abre `http://127.0.0.1:8080/`, introduce el mismo token en la UI o llama:

```bash
curl -X POST 'http://127.0.0.1:8080/search' \
  -H "Authorization: Bearer $AMATL_SERVER_TOKEN" \
  -H 'Content-Type: application/json' \
  --data '{"q":"rust"}'
```

La UI ofrece `Buscar` y `Analizar evidencia`. Ambas acciones usan POST JSON para
que la consulta no aparezca en URL, historial o access logs. Deep presenta cada
documento con fragmentos Evidence v2, linaje de URL, extractor y hashes, y
comprueba en el navegador el rango UTF-8 y SHA-256 del fragmento. El token viaja
únicamente en `Authorization`, nunca como campo del formulario ni como query
parameter. La ingestión local continúa siendo exclusiva de CLI.

Todas las respuestas de la aplicación incluyen un `X-Request-ID` generado por
AMATL para correlacionarlas con los eventos operativos y de seguridad. Los IDs
enviados por clientes no se reutilizan.

La exposición no-loopback exige en la propia configuración autenticación y un
par certificado/clave TLS completo.

## CLI

| Comando | Propósito | Salida/código |
|---|---|---|
| `amatl search "consulta" [--json]` | Search multi-provider | 0 en `success`/`partial_success`; 1 en failure/error |
| `amatl deep "consulta" [--json]` | Search + fetch/extracción acotados | 0 si la operación se entrega, incluso con degradaciones; 1 ante error de servicio |
| `amatl ingest RUTA [--query "consulta"] [--json]` | Ingestión local, despacho documental y Evidence v1/v2 | 0 si extrae evidencia; 1 ante tipo, límite, lectura o extractor fallido |
| `amatl providers` | Disponibilidad/código de providers | 0 si puede construir el resumen; 1 en error |
| `amatl provider-canary PROVIDER "consulta" [--json]` | Canario aislado y gobernado de un provider real | 0 sólo con aprobación, credencial y respuesta útil; 1 fail-closed |
| `amatl config` | Defaults/config efectiva no secreta | 0; 1 si la configuración no carga/valida |
| `amatl cache [--purge]` | Estadísticas o purga de ambas cachés | 0; storage deshabilitado se informa, no es error |
| `amatl doctor` | Diagnóstico local completo | 0 si ejecuta; estados degradados se imprimen |
| `amatl benchmark ranking-v2 [--json]` | Gate de calidad Ranking v2 | 0 si pasa; 1 si no pasa o componente inválido |
| `amatl serve` | UI + API + MCP | proceso de larga vida; 1 si config/token/TLS/listener fallan |
| `amatl mcp serve` | Alias del mismo servidor compartido | igual que `serve` |

Clap devuelve 2 ante uso sintáctico incorrecto. `--config-file RUTA` es global y
usa `amatl.toml` por defecto. Logs humanos van a stderr en TTY y JSON estructurado
al redirigir; controla detalle con `RUST_LOG`.

## Documentación

- Normas rectoras: [plan_amatl.md](plan_amatl.md) y
  [fase_a_contratos.md](fase_a_contratos.md).
- Producto: [arquitectura](docs/arquitectura.md),
  [glosario](docs/glosario.md), [configuración](docs/configuracion.md),
  [Evidence v2](docs/evidence-v2.md), [ingestión local](docs/ingestion-local.md),
  [operación](docs/operacion.md) y
  [gobernanza de providers](docs/gobernanza-providers.md).
- Contratos: [OpenAPI](docs/api/openapi.yaml) y [MCP](docs/api/mcp.md).
- Ingeniería: [desarrollo](DEVELOPMENT.md), [contribución](CONTRIBUTING.md),
  [contribución en español](docs/contribuir.md),
  [testing](docs/testing.md), [benchmarks](docs/benchmarks.md),
  [release y paquetes Linux](docs/release.md),
  [ADRs](decisiones_amatl.md) y [changelog](CHANGELOG.md).
- Seguridad: [política](SECURITY.md), [guía en español](docs/seguridad.md),
  [modelo de amenazas](docs/security/threat-model.md),
  [ASVS](docs/security/asvs-checklist.md),
  [SSRF](docs/security/ssrf-controls.md),
  [HTTP](docs/security/http-hardening.md),
  [secretos](docs/security/secrets.md),
  [cadena de suministro](docs/security/supply-chain.md) y
  [retención](docs/security/data-retention.md).
- Comunidad: [código de conducta](CODE_OF_CONDUCT.md) y
  [espejo en español](docs/codigo-de-conducta.md).
- Continuidad histórica: [CONTINUIDAD.md](CONTINUIDAD.md); no sustituye código,
  pruebas ni contratos.

## Licencia y contribuciones

AMATL se ofrece, a elección de cada usuario, bajo
[Apache License 2.0](LICENSE-APACHE) o [MIT](LICENSE-MIT), de acuerdo con
`MIT OR Apache-2.0` en Cargo. Salvo indicación explícita del contribuidor, toda
contribución enviada intencionalmente para incluirse en AMATL se ofrece bajo los
mismos términos duales, sin condiciones adicionales.

Antes de reportar una vulnerabilidad, lee [SECURITY.md](SECURITY.md). Usa GitHub
Security Advisories o el correo verificado allí; no publiques detalles sensibles
en issues. `@IA-Alex` mantiene el canal y el SLA por severidad.
