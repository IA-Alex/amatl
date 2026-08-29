Generated test artifact — SearXNG diagnostic — do not treat as project documentation.

# Trazabilidad: selección → adapter → HTTP → parsing → pipeline

## Configuración y solicitud

- `select_providers` construye el provider habilitado/aprobado y le entrega el transporte (`crates/amatl-core/src/service.rs:1280-1340`). La URL procede de la variable nombrada en la configuración runtime, si está disponible; si no, la factory usa el valor predeterminado local `http://127.0.0.1:8888` (`service.rs:1431-1437`, `providers/registry.rs:156-169`). No se inspeccionó ni registró el valor efectivo.
- `SearXngProvider::request` une `/search` y añade exclusivamente `q`, `format=json` y `pageno=1`, con `Accept: application/json` y `Cache-Control: no-cache` (`providers/searxng.rs:55-87`). Para `rust async`, el query construido es `rust async`; por tanto los parámetros son conceptualmente `q=rust async`, `format=json`, `pageno=1`.
- `SearXngProvider::search` ejecuta el transporte y sólo devuelve `ProviderError` si transporte o parsing falla (`searxng.rs:131-145`).

## Hasta dónde llegó `rust async`

`OBSERVATION`: el baseline muestra `providers_used=["searxng"]`, `providers_failed=[]`, `providers_partial=["searxng"]`, `errors=[no_usable_results]`, `degradations=[]`, `results=[]`, y `elapsed_ms=816`.

`ROOT_CAUSE` para la clasificación parcial: `execute_parallel` sólo incorpora un provider en `providers_used` cuando `provider.search` devuelve `Ok(result)`, y copia `providers_partial` exactamente si `result.status == Partial` (`crates/amatl-core/src/execution.rs:580-620`). En el adapter, `Partial` se asigna exclusivamente cuando el JSON parseado tiene al menos una entrada en `unresponsive_engines` (`providers/searxng.rs:242-320`). Por tanto, la ejecución llegó a: solicitud HTTP → respuesta 200 → UTF-8 → JSON deserializable → `ProviderResult`; y la respuesta parseada informó motores no responsivos.

No hay evidencia de un error HTTP, de transporte, de timeout o de JSON inválido en esta ejecución: cualquiera de ellos retorna `Err(ProviderError)` y se habría incluido en `providers_failed`/errores agregados, no en `providers_used` como parcial.

## Dónde quedan los resultados

El adapter transforma cada entrada de `results` y cada `answer` en `ProviderItem`; los dos vectores usan `#[serde(default)]`, de modo que un campo ausente equivale a vacío (`searxng.rs:209-240, 266-303`). El pipeline aplica, sin filtros de eliminación adicionales, `normalize → canonicalize → deduplicate → rank → diversify` (`execution.rs:672-704`).

`ROOT_CAUSE` para `no_usable_results`: la respuesta previa terminó con `results=[]` y `degradations=[]`. Un `ProviderItem` con URL ausente, inválida, no HTTP, con credenciales o host bloqueado sería descartado por `normalize` y añadiría una degradación de contrato URL (`crates/amatl-core/src/normalize.rs:13-40`; `security.rs:9-37`). Si existiera un item con URL válida, canonicalización, deduplicación y ranking conservan un elemento; `diversify` también emite un `SearchResult` por cada elemento, aunque pueda marcarlo relegado (`canonical.rs`, `dedupe.rs`, `ranking.rs:75-139`, `diversity.rs:68-150`).

Así, con la ausencia observable de degradaciones y resultados, el `ProviderResult.results` entregado al pipeline fue vacío. `SearchOrchestrator::search` marca fallo y genera `no_usable_results` cuando el resultado final está vacío y existe al menos un provider parcial (`execution.rs:373-403`).

## Qué no demuestra esta evidencia

`BLOCKED`: no permite afirmar literalmente que SearXNG devolvió `"results": []` y `"answers": []`. La misma salida interna puede proceder de campos ausentes (por los defaults de serde) o de una respuesta JSON sin items mapeables, y el cuerpo HTTP no se publica en la interfaz normal. Por ello queda demostrado que **AMATL recibió y parseó una respuesta cuyo conjunto de items mapeados era vacío**, pero no la causa upstream de ese conjunto ni el contenido bruto de la respuesta.

## Canary

`ROOT_CAUSE`: el canary primero pasa validaciones sin red (`validate_provider_canary`; `service.rs:1440-1488`) y ejecuta la misma ruta de búsqueda con sólo SearXNG habilitado. Después falla si `response.status == Failure` o SearXNG no figura en `providers_used` (`crates/amatl-cli/src/main.rs:901-919`). El baseline demuestra ambas premisas relevantes para la primera condición: la búsqueda equivalente tuvo `status=failure` y sí incluyó SearXNG. Por tanto el FAIL del canary es causado por el `SearchStatus::Failure` producido por `no_usable_results`, no por un preflight de credenciales, habilitación o gobernanza.
