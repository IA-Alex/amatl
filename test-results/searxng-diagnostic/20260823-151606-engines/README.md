Generated test artifact — SearXNG engine diagnostic — do not treat as project documentation.

# Alcance y método

- Inicio: 2026-08-23T15:16:06-07:00.
- Evidencia previa consultada y no modificada: `test-results/searxng/20260823-145832/`, `test-results/searxng-diagnostic/20260823-150442/` y `test-results/searxng-diagnostic/20260823-151035-mapping/`.
- Inspección estática: `SearXngProvider::request` únicamente añade `q`, `format=json` y `pageno=1`; no añade `engines` ni `categories`.
- Lectura de SearXNG: se leyó sólo el nombre y `disabled` de cada entrada de `/etc/searxng/settings.yml`. No se leyó ni registró ningún otro campo de configuración.
- Consulta dinámica prevista: una única repetición normal de AMATL, `rust async`, con el fixture aislado preexistente que deshabilita persistencia, historial y cachés.
- Salvaguarda antes de instrumentar: `crates/amatl-core/src/providers/searxng.rs` no tenía diff atribuible a esta prueba; SHA-256 `0f16ac5e8befdeac03fa4029d5d5588443acd2ee452c1bb27f46d7affb7b7b0d`.
- Instrumentación temporal: una sola emisión stderr desde `parse_response`, limitada a los pares públicos `(engine_name, error_type)` ya presentes en `unresponsive_engines`. No registra body, resultados, URLs, headers, cookies, tokens, variables ni credenciales.
