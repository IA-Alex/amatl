# AMATL

Herramienta Rust de búsqueda multi-provider con Search, Deep, CLI, UI, API y
MCP. Los contratos rectores son `plan_amatl.md` y `fase_a_contratos.md`.

## Inicio rápido

```bash
cp amatl.example.toml amatl.toml
cargo run -p amatl-cli -- search "rust async" --json --mock
```

Para servir UI/API/MCP:

```bash
export AMATL_SERVER_TOKEN="$(openssl rand -hex 32)"
cargo run -p amatl-cli -- serve
```

Consulta `DEVELOPMENT.md` para validación y `CONTINUIDAD.md` para retomar el
trabajo sin reconstruir decisiones previas.
