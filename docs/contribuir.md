# Contribuir a AMATL

Este es el espejo operativo en español de [`CONTRIBUTING.md`](../CONTRIBUTING.md).
También aplica el [código de conducta](../CODE_OF_CONDUCT.md); vulnerabilidades
se reportan según [`SECURITY.md`](../SECURITY.md), nunca en un issue público.

## Flujo

Parte de `main` con una rama corta `feat/`, `fix/`, `docs/` o `security/`. Usa
commits enfocados con `tipo: resumen imperativo`, acompaña código con pruebas y
documentación, y completa la plantilla de pull request. La estrategia obligatoria
de merge no está codificada en el repositorio y no debe suponerse.

El desarrollo ordinario **no modifica** `plan_amatl.md` ni
`fase_a_contratos.md`. Una propuesta contractual comienza como ADR dedicado e
incluye invariante, alternativas, compatibilidad/migración, impacto en todas las
fronteras, fixtures y contract tests. La identidad del propietario que debe
aprobarla sigue pendiente de definición.

## Definición de terminado

Todo cambio compila, mantiene la lógica en el core, no expone secretos, actualiza
documentación y pasa el gate. Provider, Canonicalization, Deduplication, Budget,
ranking, Fetcher, extractor, router y Normalization requieren contratos de
entrada válida/degradada, error tipado, parcial cuando aplique, invariantes y
Budget agotado. API/MCP/UI/storage/Deep/seguridad requieren además sus pruebas de
integración.

```bash
cargo fmt --all -- --check
cargo test --workspace
cargo check --workspace --benches
cargo clippy --workspace --all-targets -- -D warnings
cargo audit
cargo deny check
cargo cyclonedx
```

`contract-gate` debe configurarse como required check en la protección de
`main`; es una acción externa pendiente del propietario. Contribuciones
intencionales se ofrecen bajo Apache-2.0 o MIT, a elección del usuario, salvo
declaración explícita en contrario.
