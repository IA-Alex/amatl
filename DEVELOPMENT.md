# Desarrollo de AMATL

## Reglas

- No modificar los contratos rectores durante desarrollo ordinario.
- La lógica de producto vive en `amatl-core`; CLI, UI, API y MCP son bordes.
- Toda calibración debe conservar versión, rangos e invariantes contractuales.
- Nunca versionar tokens, claves, bases SQLite ni `amatl.toml` local.

## Gate local

```bash
cargo fmt --all -- --check
cargo test --workspace
cargo check --workspace --benches
cargo clippy --workspace --all-targets -- -D warnings
cargo audit --no-fetch
cargo deny check
```

Para generar el SBOM: `cargo cyclonedx`. En pull requests, el job
`contract-gate` debe configurarse como comprobación obligatoria de la rama.

## Benchmark

```bash
cargo run -p amatl-cli -- benchmark ranking-v2 --json
cargo bench -p amatl-core --bench core_contracts
```

El corpus etiquetado está en
`crates/amatl-core/benchmarks/ranking_v2_corpus.json`; cambiarlo exige revisión
del juicio de relevancia y registrar la decisión.
