# Plan 2026-11-22 — behavioral-prompt-engineering + ux-audit (con reproducción Playwright)

Entrega: informes en pantalla + archivos en `audit/`. Auditoría READ-ONLY; reparación solo tras autorización explícita.

## Alcance confirmado por el usuario
- **behavioral-prompt-engineering** sobre la superficie de comportamiento LLM de AMATL:
  - `#[tool_handler(instructions = …)]` y descripciones de tools MCP (`crates/amatl-server/src/mcp.rs`).
  - Config muerta `[strategist]` / `[answer]` en `amatl.toml` y `amatl.example.toml` tras `8f3fd080 refactor!: retire the answer/strategist feature end to end`.
- **ux-audit** sobre la UI web (`crates/amatl-ui`) con reproducción empírica en Chromium (Playwright MCP) contra el servidor local `--mock`.

## Entorno
- Servidor: `./target/release/amatl --config-file amatl.toml serve --mock` en `127.0.0.1:8080` (no_auth=true, storage off).
- Navegador: Playwright MCP (`@playwright/mcp` 0.0.84, chromium-1252).

## Fases
- [x] Fase 0 — Contexto: crates, UI completa, superficie MCP, config, commits recientes
- [x] Fase 0b — Entorno vivo: build release OK, servidor `--mock` levantado, Chromium MCP conectado
- [x] Fase 1 — ux-audit: Hunter (5 ejes) sobre `index.html`/`app.js`/`styles.css` + reproducción Playwright
- [x] Fase 2 — ux-audit: Skeptic (refutación) + Referee (veredicto/severidad) + variantes → `audit/auditoria-ux-conductual-2026-11-22.md`
- [x] Fase 3 — behavioral-prompt-engineering: perfilado, 6 capas, red-team, evals → `audit/auditoria-behavioral-prompt-2026-11-22.md`
- [x] Fase 4 — Informes finales en pantalla + `audit/` (contrato JSON para reparación)
- [x] Fase 5 — Reparación (autorizada por el usuario): UX-001..004 + BP-001..002 aplicadas y verificadas en vivo

## Restricciones
- Read-only hasta autorización. No tocar `amatl.toml`/`.env`/SQLite sin permiso.
- Informes de auditoría en español (convención del repo).
