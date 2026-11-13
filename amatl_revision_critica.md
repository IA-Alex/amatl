# Amatl — revisión crítica independiente del archivo comprimido

## Alcance y límites
Fuente: `amatl.tar(2).xz`, extraído y examinado en copia aislada. HEAD archivado `365b5836`, rama `chore/start-sh-token-bootstrap`, 19 commits adelante de `origin/chore/start-sh-token-bootstrap` según referencias *incluidas en el archivo*. No se consultó el remoto real ni se ejecutó push. El contenedor de revisión no tiene `cargo` ni `rustc`; no se reproducen las 681 pruebas ni se acredita CI remoto. No se modifica el repositorio original.

## Hallazgos

### H1 — Error material en el informe de validación (confirmado)
La fila «Sólo orígenes deshabilitados (declarados)» = `/ready` 200 contradice la implementación real si ningún origen está habilitado. `crates/amatl-core/src/service.rs` calcula `enabled_usable` únicamente cuando `summary.enabled && status == Available && circuit == Closed`; `degraded = summaries_unavailable || (storage.enabled && !storage.available) || !enabled_usable`. `crates/amatl-server/src/tests.rs::ready_reports_unavailable_when_no_source_is_enabled` exige HTTP 503. **Corrección del informe**: «Un origen habilitado utilizable + otros declarados deshabilitados» => 200; «ningún origen habilitado» => 503. No es un defecto de D2; es un error en la tabla narrativa.

### H2 — Semántica de `sources_available` no equivale a fuentes utilizables (confirmado en código, impacto por determinar)
`crates/amatl-server/src/lib.rs::ready` cuenta los `SourceStatus` con `ProviderSurfaceStatus::Available`, pero no excluye circuitos abiertos; `service.rs` sí excluye circuitos abiertos para decidir readiness. Por tanto es posible `/ready` HTTP 503, `status=degraded`, `sources_available=1` cuando la única fuente aparece Available con circuito abierto. El campo se denomina *available*, no *usable*: puede ser un contrato intencional, pero induce interpretaciones erróneas en automatización. No cambiar su significado sin revisar consumidores y pruebas; documentar expresamente la diferencia o introducir un campo versionado en otra intervención autorizada.

### H3 — El gate local no acredita CI remoto (confirmado)
`.github/workflows/ci.yml` define jobs `contract-gate`, `msrv`, `cross-platform`, `browser-e2e`, `trafilatura-integration` y `soak` condicionado a cron/manual. Hay otros workflows `static-analysis`, `codeql-invariants`, `chromium-isolation`. Ninguno se ha ejecutado desde esta revisión; la aprobación local reportada no los sustituye. En especial, macOS/Windows, Chrome WebDriver, MSRV y CodeQL no se validan con `cargo test --workspace` en una sola máquina.

### H4 — Publicación remota no autorizada ni comprobada (confirmado)
`.git/config` contiene `origin` a `https://github.com/IA-Alex/amatl.git`; la referencia remota archivada puede estar desactualizada. Los 19 commits ahead son relativos a esa referencia archivada. Debe consultarse `git ls-remote`/GitHub antes de decidir el push; no hacer push ni crear tags automáticamente.

### H5 — Advertencia de dependencia (deuda documentada)
El informe adjunto reconoce `chacha20 0.10.1` yanked, transitivo a través de `rand`/`rmcp`. Los gates locales informados aceptaron esa advertencia; no se demuestra remediación. No actualizar el lockfile a ciegas.

> **Corrección 2026-11-13:** H5 es obsoleto. El lockfile actual ya fija `chacha20 0.10.2` (remediado el 2026-10-26, documentado en `docs/security/supply-chain.md`); `git show ce172d9d:Cargo.lock` ya traía `0.10.2`, así que la advertencia citada no existía ni en la base de esta revisión. Se deja el párrafo literal como registro histórico y se remite a la re-auditoría del 2026-11-13, donde el riesgo vigente es otro (`libsqlite3-sys` RUSTSEC-2026-0459, abierto y sin fix compatible).

### H6 — CI con efectos distintos según evento (confirmado)
El workflow de contrato se activa por push/PR; `release.yml` se activa en `main` para ciertos paths, tags RC y manual; `publish-crates.yml` y `publish-aur.yml` admiten tags estables y despacho manual. Una publicación de rama y la creación de una etiqueta NO son operaciones equivalentes. Evitar etiquetas y despachos manuales sin aprobación específica.

## Comprobaciones realizadas sobre la copia
- Integridad Git: HEAD `365b5836`, rama y referencias archivadas coherentes.
- `python3 tools/check_doc_refs.py`: `12 known-dead, 0 new`, exit 0.
- `python3 -m compileall -q tools`: exit 0 (sintaxis Python, no prueba funcional).
- `git diff --check`: sin errores en copia.
- Código de D1: módulo de adapter del proveedor de pago retirado eliminado; `provider-canary.yml` ofrece `searxng` y `marginalia`.
- Código de D2: comprobado contra `service.rs` y pruebas HTTP reales del repositorio.

## Decisión
D1 y D2 no requieren una nueva modificación a partir de los hallazgos confirmados. Corregir la tabla del informe, no la lógica. El pendiente prioritario es CI remoto asociado al SHA exacto y la autorización explícita del destino de publicación. La semántica de `sources_available` es una mejora de contrato/documentación que no debe mezclarse con el cierre ya probado sin validar consumidores.

## Lo que no se puede afirmar
No se ha ejecutado Rust/Cargo en este contenedor, no se ha comprobado el estado actual de GitHub, no se han ejecutado GitHub Actions y no se han verificado integraciones externas. No procede declarar producción validada ni un GO remoto incondicional.
