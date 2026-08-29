Generated test artifact — SearXNG mapping diagnostic — do not treat as project documentation.

# Hallazgos

## ROOT_CAUSE — Caso A

La única ejecución de `rust async` midió `http_status=200`, `results.len()=0`, `answers.len()=0`, `unresponsive_engines.len()=2` y `mapped_items.len()=0`. El resultado público final fue cero resultados utilizables, `no_usable_results`, sin degradaciones.

La primera etapa donde aparece cero es el vector `results` deserializado de SearXNG. Por tanto, en esta ejecución SearXNG entregó cero resultados al adapter de AMATL. El adapter no eliminó resultados: no tenía resultados ni respuestas para mapear.

## OBSERVATION

- El HTTP status fue 200 y el provider quedó parcial; el conteo de motores no responsivos fue 2.
- No se registraron nombres de motores, body, contenidos, URLs, headers ni secretos.
- Se ejecutó exactamente una consulta y ninguna solicitud directa a SearXNG.

## BLOCKED / sin determinar

Esta evidencia no determina por qué SearXNG devolvió cero resultados ni por qué informó dos motores no responsivos. No permite atribuir la causa a un motor, a la configuración de la instancia o a una condición upstream.

## Instrumentación y reversión

El único cambio temporal estuvo en `crates/amatl-core/src/providers/searxng.rs`, función `parse_response`: tres variables locales de conteo y una línea stderr con sólo los cinco conteos permitidos. Fue retirado tras la medición. El SHA-256 final coincide exactamente con el previo (`0f16ac5e8befdeac03fa4029d5d5588443acd2ee452c1bb27f46d7affb7b7b0d`) y `git diff -- crates/amatl-core/src/providers/searxng.rs` quedó vacío. El binario fue recompilado tras retirar la instrumentación.
