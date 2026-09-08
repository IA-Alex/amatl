# Desarrollo de AMATL

## Toolchain

El workspace usa Rust 2021 y fija `rust-version = "1.88"`, el mayor MSRV del
grafo actual (`rmcp`, `time` y macros relacionadas). CI verifica tanto `stable`
como un job explícito con Rust 1.88. La línea base local fue verificada con
`rustc/cargo 1.97.1`.

Herramientas requeridas: `cargo-audit`, `cargo-deny` y `cargo-cyclonedx`.
Trafilatura 2.2.0 es opcional y sólo mejora Deep; CI valida su CLI real.
Chromium tampoco es dependencia base: `packaging/amatl-chromium-sandbox` prueba
el aislamiento Linux sin red, pero el Renderer del core permanece desactivado
hasta disponer de un bridge CDP que conserve el ownership de `SafeFetcher`.

## Layout

```text
crates/amatl-core/    contratos, pipeline, providers, Search y Deep
crates/amatl-cli/     binario amatl y adaptación de salida
crates/amatl-ui/      assets estáticos embebidos y headers
crates/amatl-server/  Axum API/UI/MCP y hardening HTTP
docs/                 arquitectura, contratos, operación y seguridad
.github/              CI y plantillas de colaboración
```

La lógica de producto vive en `amatl-core`. `plan_amatl.md` y
`fase_a_contratos.md` no se modifican en desarrollo ordinario. Nunca se
versionan `amatl.toml`, bases SQLite ni secretos.

## Ejecutar superficies

```bash
# CLI Search/Deep determinista sin red real
cargo run -p amatl-cli -- search "rust async" --json --mock
cargo run -p amatl-cli -- deep "rust async" --json --mock

# Config, providers, cache y diagnóstico
cargo run -p amatl-cli -- config
cargo run -p amatl-cli -- providers
cargo run -p amatl-cli -- cache
cargo run -p amatl-cli -- doctor

# Sólo después de aprobación y credencial: canario de un único provider real
cargo run --release -p amatl-cli -- --config-file amatl.toml \
  provider-canary brave "rust programming language" --json

# UI + API + MCP en un listener
export AMATL_SERVER_TOKEN="$(openssl rand -hex 32)"
cargo run -p amatl-cli -- serve --mock
```

`amatl mcp serve` arranca el mismo servidor; MCP vive en `/mcp`, no en otro
daemon. Usa `--config-file RUTA` antes o después del subcomando por ser argumento
global.

## Depuración

```bash
RUST_LOG=amatl=debug cargo run -p amatl-cli -- search "rust" --mock
RUST_LOG=amatl_core=trace,amatl_server=debug cargo run -p amatl-cli -- serve --mock
```

En TTY los logs de stderr son compactos y humanos. Redirigidos, son JSON con
`ts`, `level`, `target`, `msg` y `context`. No añadas consultas, tokens, headers
de autenticación, cookies ni URLs no sanitizadas a eventos. stdout pertenece al
contrato de salida CLI.

## Artefactos de compilación

Cargo concentra fuera del código fuente los artefactos temporales en
`target/` (por defecto, `<raíz-del-repositorio>/target/`). Ahí quedan las
dependencias compiladas, binarios de `debug` y `release`, ejecutables de tests,
cachés incrementales y resultados auxiliares de benchmarks. Una ejecución
repetida de `cargo test --workspace --all-targets`, Clippy y benchmarks puede
hacer que esta carpeta ocupe decenas de GiB.

`target/` está ignorado por Git: no contiene configuración, fuentes, secretos
ni las bases SQLite de AMATL. Para recuperar espacio, con ningún binario de
AMATL ejecutándose desde esa ruta, usa:

```bash
cargo clean
```

La siguiente compilación será más lenta. Si se necesita volver a ejecutar el
binario directo, reconstruye primero con `cargo build --release`; el binario
queda en `target/release/amatl`.

## Añadir un provider

1. Completa y aprueba la ficha descrita en
   `docs/gobernanza-providers.md`; ToS, coste y derechos preceden al código.
2. Implementa `Provider` en `crates/amatl-core/src/providers/`. El router
   recomienda; el adapter no reinterpreta texto libre, no modifica SearchPlan y
   no asigna Budget.
3. Declara capabilities exactas y disponibilidad fail-closed. Lee credenciales
   por el nombre de variable de entorno; nunca desde TOML.
4. Usa el transporte acotado, errores `ProviderError` tipados, parsing estructural
   y sanitización. No sigas redirects ni registres requests con secretos.
5. Añade tests unitarios del mapping/parser y contract tests de éxito, parcial,
   auth, rate limit, timeout, respuesta inválida, filtros aceptados/ignorados y
   governance gate.
6. Integra el resumen y la selección en `AmatlService`; comprueba cache sólo si
   `storage_rights` está verificado.

## Gate local idéntico a CI

```bash
cargo fmt --all -- --check
cargo test --workspace
cargo check --workspace --benches
cargo clippy --workspace --all-targets -- -D warnings
cargo audit
cargo deny check
cargo cyclonedx
```

`contract-gate` debe ser required check en la protección de `main`; esa
protección requiere hacer público el repositorio o actualizar el plan actual de
GitHub. Hasta entonces, el gate verde es obligatorio como evidencia de revisión,
pero GitHub no lo impone.

## Benchmarks

```bash
cargo run -p amatl-cli -- benchmark ranking-v2 --json
cargo run --locked --release -p amatl-cli -- \
  benchmark operational --json --iterations 64 --concurrency 8
cargo bench -p amatl-core --bench core_contracts
```

El corpus humano está en
`crates/amatl-core/benchmarks/ranking_v2_corpus.json`. Cambiar consulta,
documento, relevancia o provider rank exige revisión del juicio y registro ADR.
La metodología y los huecos operativos están en `docs/benchmarks.md`.

## Tooling de autor

El repositorio no obliga editor ni asistente. VS Code, Claude Opus y DeepSeek Pro
son tooling externo, no dependencias del producto ni fuentes normativas. Código
generado con cualquier herramienta debe revisarse, atribuir licencias, excluir
secretos y pasar exactamente el mismo gate.
