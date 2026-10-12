# Documentación de AMATL

Este es el **portal de la documentación** del proyecto. Agrupa todo lo que vive
bajo `docs/` y lo que depende de él, organizado por el rol que juega cada pieza
(Diátaxis: *Explanation*, *How-to*, *Reference*) más los dominios **Seguridad**,
**Contratos/API**, **Testing y evaluación** y **Experimentos**.

> Regla de oro: la **fuente de verdad** de la arquitectura y las decisiones es el
> código y sus contratos; la documentación la explica y la referencia. Si un
> documento contradice el código, manda el código (y el documento debe corregirse).

## Política de idioma

- El idioma primario de esta documentación es el **español** (ES).
- Las guías de comunidad centradas en GitHub (`CONTRIBUTING`, `SECURITY`,
  `CODE_OF_CONDUCT`) tienen una fuente canónica en inglés y un **espejo traducido
  en español** vinculado bidireccionalmente. Cuando ambos existan, cámbialos
  juntos — no deriven por separado.
- Los contratos de máquina (`openapi.yaml`, `mcp.md`), los artefactos de testing
  y los experimentos pueden permanecer en inglés por ser consumidos por
  herramientas/procesos, pero los documentos explicativos de producto van en
  español.

| Fuente (canónica) | Espejo (ES) | Nota |
|---|---|---|
| `README.md` | – | Entrada del repo |
| `CONTRIBUTING.md` (EN) | `docs/contribuir.md` | Espejo traducido |
| `SECURITY.md` (EN) | `docs/seguridad.md` | Espejo traducido |
| `CODE_OF_CONDUCT.md` (EN) | `docs/codigo-de-conducta.md` | Espejo traducido |

## Índice

### Normas rectoras (Explanation de alto nivel)

Definen el *porqué* y el contrato de largo plazo; no son guías operativas.

| Documento | Contenido |
|---|---|
| [plan_amatl.md](../plan_amatl.md) | Especificación canónica y normas rectoras (golden template) |
| [fase_a_contratos.md](../fase_a_contratos.md) | Fase A: contratos ejecutables |
| [decisiones_amatl.md](../decisiones_amatl.md) | ADRs — decisiones de arquitectura y sus trade-offs |
| [CONTINUIDAD.md](../CONTINUIDAD.md) | Continuidad de desarrollo; no sustituye código ni contratos |

### Arquitectura y producto (Explanation)

| Documento | Contenido |
|---|---|
| [arquitectura.md](arquitectura.md) | Vista arquitectónica: workspace, boundaries, ownership, invariantes |
| [glosario.md](glosario.md) | Vocabulario canónico |
| [evidence-v2.md](evidence-v2.md) | Fragmentos verificables y procedencia |
| [ingestion-local.md](ingestion-local.md) | Ingestión local y despacho documental |

### Configuración y operación (How-to / Reference)

| Documento | Contenido |
|---|---|
| [configuracion.md](configuracion.md) | Referencia de configuración (`amatl.toml`) |
| [operacion.md](operacion.md) | Operación del servicio: arranque, recarga, TLS, observabilidad |
| [gobernanza-providers.md](gobernanza-providers.md) | Aprobación y gobernanza de providers |
| [resumen-con-ia.md](resumen-con-ia.md) | Sintetización `answer` ("Resumen con IA") |
| [identidad-visual.md](identidad-visual.md) | Tokens visuales e identidad de marca |
| [ux-ui-spec.md](ux-ui-spec.md) | Especificación UX/UI y [wireframe](ux-wireframe.svg) |

### Seguridad

| Documento | Contenido |
|---|---|
| [SECURITY.md](../SECURITY.md) | Política de seguridad y reporte de vulnerabilidades (EN) |
| [seguridad.md](seguridad.md) | Guía de seguridad en español (espejo) |
| [security/threat-model.md](security/threat-model.md) | Modelo de amenazas |
| [security/asvs-checklist.md](security/asvs-checklist.md) | Checklist ASVS |
| [security/secrets.md](security/secrets.md) | Gestión de secretos |
| [security/ssrf-controls.md](security/ssrf-controls.md) | Controles SSRF |
| [security/http-hardening.md](security/http-hardening.md) | Hardening HTTP/TLS |
| [security/data-retention.md](security/data-retention.md) | Retención de datos |
| [security/supply-chain.md](security/supply-chain.md) | Seguridad de la cadena de suministro |
| [security/chromium-isolation.md](security/chromium-isolation.md) | Aislamiento del renderer Chromium |

### Contratos (API y transporte)

| Documento | Contenido |
|---|---|
| [api/openapi.yaml](api/openapi.yaml) | Contrato HTTP (OpenAPI) |
| [api/mcp.md](api/mcp.md) | Model Context Protocol |

### Ingeniería (How-to de desarrollo)

| Documento | Contenido |
|---|---|
| [DEVELOPMENT.md](../DEVELOPMENT.md) | Desarrollo y build |
| [CONTRIBUTING.md](../CONTRIBUTING.md) | Contribución (EN) |
| [contribuir.md](contribuir.md) | Contribución (espejo ES) |
| [release.md](release.md) | Release y empaquetado Linux |
| [benchmarks.md](benchmarks.md) | Benchmarks y quality gates |
| [testing.md](testing.md) | Estrategia de testing y cobertura |

### Testing, evaluación y experimentos (evidencia)

| Documento | Contenido |
|---|---|
| [testing/](testing/) | Estudios de testing puntuales (relevance, searxng) |
| [evaluation/](evaluation/) | Estudios de evaluación (protocols, step4e) |
| [experiments/](experiments/) | Experimentos técnicos (candle, embeddings) |

### Comunidad (normas de convivencia)

| Documento | Contenido |
|---|---|
| [CODE_OF_CONDUCT.md](../CODE_OF_CONDUCT.md) | Código de conducta (EN) |
| [codigo-de-conducta.md](codigo-de-conducta.md) | Código de conducta (espejo ES) |
| [LICENSE-APACHE](../LICENSE-APACHE) / [LICENSE-MIT](../LICENSE-MIT) | Licencias duales |

## Notas de mantenimiento

- **No** crear un documento suelto en la raíz si su rol es de referencia/operación:
  añadir a `docs/` y registrarlo aquí.
- Mantener la **política de idioma**: si un documento tiene espejo, actualízalos
  juntos o vincúlalos.
- Los `ADRs` viven en `decisiones_amatl.md`; las decisiones nuevas se añaden ahí
  (no en un archivo suelto) salvo que el volumen justifique dividirlo.
- Antes de eliminar o renombrar un documento, actualizar este portal y el
  `README.md` raíz para no dejar enlaces rotos.
