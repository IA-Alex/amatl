# Diagramas de arquitectura (2026-11-18)

Fuente única: el código de `crates/` y `docs/api/`. Todos son Mermaid (UML-like:
C4 + diagramas de secuencia/clases-flujo), generados por la pasada
bug-hunter → api-developer → UML del 2026-11-18 (`audit/informe-2026-11-18.md`).

| Fichero | Tipo | Responde a |
|---|---|---|
| `c4-contexto.mmd` | C4 L1 (contexto) | Quién usa AMATL y de qué sistemas externos depende |
| `c4-contenedores.mmd` | C4 L2 (contenedores) | Cómo se reparte la responsabilidad entre crates y storage |
| `secuencia-busqueda.mmd` | Secuencia UML | Recorrido completo de `POST /search` (middleware → orquestador → providers → pipeline) |
| `secuencia-deep.mmd` | Secuencia UML | Deep: única frontera de fetch de red (SafeFetcher, evidence_v2) |
| `secuencia-mcp.mmd` | Secuencia UML | Llamada MCP: identidad autenticada, allowlist de tools, caps `for_surface` |
| `flujo-pipeline-busqueda.mmd` | Flujo UML-like | Orden real de `run_pipeline` y estados `success/partial_success/failure` |

## Notaciones y decisiones

- **Recomendada:** secuencia UML para los recorridos de request (orden de mensajes a través de actores reales) y C4 L1/L2 para estructura; **respaldo:** diagrama de componentes UML formal si se necesitara intercambio XMI/UMLDI (no es el caso: el objetivo es documentación viva).
- Etiquetas en español, identificadores técnicos tal cual (`SearchOrchestrator`, `SafeFetcher`, `ErrorCode`…). Los tipos y módulos citados existen en el código; las relaciones de dependencia están tomadas de los `use`/llamadas reales, no inferidas.
- Sin relaciones "inferidas" destacadas: si una flecha es opcional (Trafilatura, Chromium, completado remoto) se dibuja punteada.

## Cómo renderizar

```bash
# Con mermaid-cli (npx descarga @mermaid-js/mermaid-cli la primera vez)
for f in docs/diagrams/*.mmd; do npx -y @mermaid-js/mermaid-cli -i "$f" -o "${f%.mmd}.svg" -b white; done
```

También se previsualizan tal cual en GitHub/GitLab (basta con abrir el `.mmd`)
y en cualquier visor Mermaid. Los `.svg` renderizados se tratan como artefacto
derivado: el `.mmd` es la fuente.

## Mantenimiento

- Al cambiar una ruta pública, el flujo Deep/MCP o el pipeline, actualizar aquí y
  el informe de auditoría correspondiente en `audit/`.
- Los guards del contrato (tests de `amatl-server/src/tests.rs`) protegen el
  inventario de rutas y el catálogo de errores; los diagramas se revisan a mano
  (son documentación, no contrato ejecutable).