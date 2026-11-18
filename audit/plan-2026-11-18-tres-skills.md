# Plan 2026-11-18 — bug-hunter → api-developer → UML (workspace completo)

Entrega: en pantalla + archivos. Solo artefactos de auditoría/diagramas; código solo si se autoriza reparación.

## Fases

- [x] Fase 0 — Contexto: memorias Serena, mapa de crates, auditoría previa 2026-11-13
- [x] Fase 0b — Baseline dinámico: `cargo test --workspace --locked` (25 suites OK, 0 fallos), `cargo audit` (1 vuln: libsqlite3-sys RUSTSEC-2026-0459), hallazgos previos verificados (GET /deep remediado, MCP docs = 6 tools)
- [x] Fase 1 — bug-hunter: 10 hallazgos (5 CONFIRMED, 2 PROBABLE, 2 POTENTIAL, 1 NOT_A_BUG) → `audit/bug-hunter-2026-11-18.md` + 2 informes de frente con reproducciones
- [x] Fase 2 — api-developer: revisión de contrato HTTP + MCP → `audit/api-developer-2026-11-18.md` (veredicto: EXCELENTE/APTO + 5 ajustes propuestos)
- [x] Fase 3 — UML: 6 diagramas Mermaid + 6 SVG en `docs/diagrams/` (C4 L1/L2, secuencias search/deep/MCP, flujo pipeline) + README
- [x] Cierre — dictamen consolidado en `audit/informe-2026-11-18.md`
- [ ] Cierre — reportes finales en pantalla + `audit/informe-2026-11-18.md`

## Salidas previstas

- `audit/bug-hunter-2026-11-18.md` — hallazgos clasificados (CONFIRMED/PROBABLE/POTENTIAL/NOT_A_BUG/ENVIRONMENTAL)
- `audit/api-developer-2026-11-18.md` — revisión de contrato + propuesta
- `docs/diagrams/*.mmd` (+ SVG si hay renderer) — C4 y secuencias
- `audit/informe-2026-11-18.md` — dictamen consolidado

## Restricciones

- Investigación READ-ONLY (bug-hunter): no se repara nada sin autorización explícita.
- No tocar `plan_amatl.md`, `fase_a_contratos.md`, `amatl.toml`, `.env`, SQLite.
- Convenciones del repo: código/doc técnica en inglés donde corresponda; informes de auditoría en español.