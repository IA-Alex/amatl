Generated test artifact — SearXNG mapping diagnostic — do not treat as project documentation.

# Alcance y método

- Inicio: 2026-08-23T15:10:35-07:00.
- Objetivo: medir solamente `http_status`, `results.len()`, `answers.len()`, `unresponsive_engines.len()` y `mapped_items.len()` en una ejecución normal de AMATL.
- Consulta única prevista: `rust async`.
- Configuración de ejecución: el fixture aislado preexistente `test-results/searxng/20260823-145832/amatl-isolated.toml`; mantiene persistencia, historial y cachés deshabilitados. No se modifica.
- Evidencia previa consultada y no modificada: `test-results/searxng/20260823-145832/` y `test-results/searxng-diagnostic/20260823-150442/`.
- Salvaguarda previa: `crates/amatl-core/src/providers/searxng.rs` estaba sin diff atribuido a esta prueba y su SHA-256 era `0f16ac5e8befdeac03fa4029d5d5588443acd2ee452c1bb27f46d7affb7b7b0d`.
- Instrumentación prevista: una única emisión temporal a stderr inmediatamente antes de retornar `ProviderResult` en `parse_response`; no registra body, URLs, headers, resultados, credenciales ni nombres de motores.
- Reversión prevista: retirar exactamente esa emisión y sus variables de conteo; comparar el SHA-256 final y `git diff` del archivo contra la salvaguarda.
