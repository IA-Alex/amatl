# Evidence v2: fragmentos verificables y procedencia

Evidence v2 es una extensión aditiva de Deep. Su objetivo es que un consumidor
pueda presentar o procesar fragmentos relevantes sin perder el vínculo con el
documento recuperado y sin pedir a un LLM que invente citas. El
`schema_version` global continúa en `"1"`; `evidence_version = "v2"` versiona
esta capacidad concreta.

## Compatibilidad y ownership

- `DeepResponse.evidence` conserva Evidence v1 sin cambios.
- `DeepResponse.evidence_v2` añade un elemento por cada `Document`.
- Ranking v2 y Gap Analyzer siguen consumiendo Evidence v1. Evidence v2 copia
  exactamente `evidence_score` y expone sus componentes en `score_basis`; no
  recalibra ni altera el ranking sin benchmark.
- Search no conoce ni produce fragmentos. El trabajo permanece dentro de Deep.
- La extracción es determinista y local; no requiere inferencia ni red extra.
- La ingestión local reutiliza el mismo análisis sobre documentos `file:` y
  marca `fetch_method = local`; no fabrica un resultado Search.

## Contrato

Cada `EvidenceV2` contiene:

- `document_id`, estado y score heredados del análisis v1;
- una `provenance` con `original_url → canonical_url → final_url`, método de
  fetch, extractor, tiempos y hashes del cuerpo recuperado y del texto extraído;
- hasta ocho `fragments`, de 512 bytes como máximo cada uno;
- `score_basis`, que mantiene separadas las señales de calidad de evidencia.

Cada fragmento incluye texto exacto, offsets UTF-8 `start_byte`/`end_byte`
sobre `Document.content`, `fragment_hash`, términos coincidentes y señales
`query_match`, `citation`, `temporal` y `numeric`. `ordinal` empieza en uno y
representa el orden original entre los fragmentos seleccionados.

`provenance_id` identifica de forma estable la combinación de documento, linaje
de URL, hashes, fetch, extractor y fechas. `fragment_id` combina esa procedencia,
rango y hash. Dos fuentes con el mismo texto tienen el mismo `fragment_hash`,
pero distintos `fragment_id`; así pueden detectarse coincidencias sin confundir
su origen.

## Selección determinista

1. El texto extraído se divide por párrafos y, cuando supera 512 bytes, por el
   último límite de oración o espacio disponible sin romper UTF-8.
2. Cada candidato recibe prioridad por términos de la consulta, URL citada,
  fecha con forma ISO y cifras observables. No se infieren entidades ni
  afirmaciones.
3. Se conservan los ocho candidatos de mayor prioridad, con desempate por
   offset; la salida se reordena por posición original.
4. Un documento superficial conserva procedencia y score parcial, pero devuelve
   `fragments: []`: AMATL no inventa texto ausente.

## Verificación y límites

Un consumidor puede verificar `Document.content[start_byte..end_byte] == text`
y SHA-256 de `text == fragment_hash`. `extracted_content_hash` valida el campo
completo; `source_content_hash` conserva el hash del cuerpo recuperado que ya
pertenece a `Document`.

Los fragmentos son contenido externo no confiable: deben mostrarse como texto,
nunca como HTML ejecutable. Los límites impiden que Evidence v2 multiplique sin
cota el tamaño de Deep. La respuesta puede contener información sensible del
documento; aplica la misma política de acceso, logs y retención que a
`Document.content`.
