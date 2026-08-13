# Ingestión local y despacho documental

`amatl ingest` convierte un archivo local explícito en `Document`, Evidence v1
y Evidence v2. No ejecuta Search, no consulta providers, no requiere LLM y no
está disponible por UI, HTTP o MCP.

```bash
amatl ingest ./informe.md --query "controles de seguridad" --json
amatl ingest ./evidencia.csv
```

`--query` es opcional. Cuando existe, sólo prioriza fragmentos coincidentes de
Evidence v2; no cambia el texto extraído ni envía información fuera del host.

## Despacho

| Tipo | Detección | Tratamiento |
|---|---|---|
| texto | `.txt`, `.text`, `.log`, `.rst` o UTF-8 desconocido | preservación local |
| Markdown | `.md`, `.markdown`, `.mdx` | preservación local |
| HTML | `.html`, `.htm`, `.xhtml` o firma HTML | texto visible; excluye `script`, `style`, `noscript`, `template` y `head` |
| JSON | `.json` | validación y formato determinista |
| JSON Lines | `.jsonl`, `.ndjson` | validación por línea y normalización compacta |
| tabular | `.csv`, `.tsv` | preservación UTF-8 |
| código/configuración | extensiones conocidas de Rust, Python, JS/TS, Java, Go, C/C++, C#, Ruby, PHP, shell, SQL, TOML y YAML | preservación UTF-8 |
| PDF | firma `%PDF-` | proceso local `pdftotext`, sólo cuando la política permite crear el extractor externo |

La firma PDF tiene prioridad sobre la extensión. Un `.pdf` sin firma válida,
JSON inválido, texto no UTF-8, binario desconocido, directorio, archivo vacío o
tipo no soportado falla de forma explícita. Archivos comprimidos y formatos de
ofimática como DOCX todavía no se descomprimen ni interpretan.

## Límites y política

- un archivo regular por ejecución;
- máximo 20 MiB de entrada y 8 MiB de texto extraído;
- máximo 8 segundos para `pdftotext`;
- stdout del extractor PDF acotado, stderr descartado y proceso terminado al
  expirar;
- no hay persistencia ni uso de caché documental;
- la salida JSON contiene URI `file:` absoluta, cuerpo extraído y evidencias:
  debe tratarse como información sensible.

Con `profile = "isolated"` o `egress = "deny"`, PDF falla antes de crear el
proceso externo porque AMATL no puede demostrar por sí solo que ese ejecutable
carece de red. Los extractores en proceso siguen disponibles. El firewall o
sandbox del host continúa siendo necesario para aislamiento verificable.

`content_hash` cubre los bytes del archivo y `extracted_content_hash` el texto
despachado. `fetch_method = "local"` distingue esta adquisición de HTTP y
renderizado. Evidence v2 conserva offsets exactos sobre `Document.content`.
