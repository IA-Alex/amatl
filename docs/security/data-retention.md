# Retención y tratamiento de datos

La persistencia SQLite, ambas cachés y la telemetría persistente están
desactivadas por defecto. Search conserva su correctness sin SQLite; abrir o
usar la base es best effort (`service.rs:101-112`).

## Inventario

| Clase | Contenido | Default | Retención/límite | Borrado |
|---|---|---:|---|---|
| Caché de provider | provider, versión de adapter, consulta normalizada, filtros estructurados, `ProviderResult`, tamaño y marcas temporales | desactivada | TTL 300 s; 10,000 entradas; 256 MiB; LRU | Expirados se excluyen al leer y se eliminan al escribir; `amatl cache --purge` vacía la tabla |
| Caché documental | URL canónica, hash, versión de extractor, `Document` serializado, tamaño y marcas temporales | desactivada | TTL 86,400 s; 1,000 entradas; 256 MiB; LRU | Expirados se excluyen al leer; cuota se aplica al escribir; `amatl cache --purge` vacía la tabla |
| Contenido documental | campo `Document.content` dentro de la caché documental | `store_content = false` | sólo si se habilita explícitamente, sujeto al TTL/cuota documental | misma purga de caché documental |
| Fragmentos Evidence v2 | vistas exactas y acotadas derivadas de `Document.content`, con hashes y procedencia | sólo respuesta en memoria | no se persisten como entidad independiente | desaparecen al terminar la operación; si el documento se almacena, aplica la política de contenido documental |
| Ingestión local | URI `file:`, nombre, texto extraído, hashes, metadata del filesystem y Evidence | sólo respuesta CLI en memoria | no usa caché documental ni SQLite | desaparece al terminar el proceso; redirección de stdout queda bajo control del operador |
| Telemetría | timestamp, provider, categoría, outcome, latencia, conteos, ratio de duplicados, contribución top-K, diversidad y coste | sólo memoria; SQLite desactivado | ventana/retención fija v1 de 30 días; decaimiento exponencial con vida media de 30 días | memoria y SQLite eliminan observaciones anteriores a la ventana al registrar; no hay comando CLI de purga dedicado |

Fuentes: `config.rs:382-483`, migraciones `0001_phase3_persistence.sql` y
`0002_phase5_document_cache.sql`, `storage.rs:121-451`, `telemetry.rs:8-10,
114-134,372-452`.

Los payloads de provider/documento incluyen el `schema_version` de sus tipos.
Las filas normalizadas de telemetría no tienen actualmente una columna
`schema_version`; su forma depende de la migración SQLite (`user_version = 2`).
Esto debe evaluarse antes de una migración incompatible y no debe confundirse
con el contrato JSON `"1"`.

## Condiciones para escribir

La caché de provider sólo lee/escribe cuando está habilitada **y** la ficha del
provider declara `storage_rights = true` (`cache.rs:39-90,139-159`). La caché
documental exige estar habilitada y que todos los providers del resultado
declaren derechos; el mock y providers desconocidos no los obtienen
(`service.rs:188-207`, `document_cache.rs:47-75`). La clave documental contiene
URL canónica, hash de contenido y versión del extractor.

Si `store_content` es falso, se persiste metadata del `Document` pero se elimina
su cuerpo antes de serializar (`document_cache.rs:47-60`). Configurar una caché o
telemetría persistente sin `persistence.enabled = true` es inválido
(`config.rs:570-580`).

Evidence v2 no abre una ruta de almacenamiento adicional: sus fragmentos se
calculan durante Deep a partir de `Document.content` y sólo forman parte de la
respuesta. Un consumidor debe tratarlos con la misma confidencialidad que el
documento original; los hashes facilitan verificación e identidad, no cifrado.

`amatl ingest --json` incluye una URI `file:` absoluta y puede incluir el cuerpo
completo extraído. No adjuntes esa salida a tickets o logs sin revisar ruta y
contenido. La ingestión no escribe el documento en SQLite y no está expuesta en
API/MCP.

## Datos que no deben almacenarse

Nunca deben llegar a configuración, SQLite, caché, telemetría o logs: token del
servidor, claves de provider, passwords, cookies o cabeceras Authorization. La
telemetría no contiene texto de consulta ni cuerpos. Fuera de la caché documental
habilitada explícitamente no se persiste contenido completo. Las respuestas se
mantienen en memoria durante la operación y se entregan al cliente.

La base SQLite no cifra datos; su confidencialidad y respaldo dependen de los
permisos y políticas del host. No hay sincronización externa implementada.

## Purga

```bash
amatl --config-file amatl.toml cache --purge
```

El comando requiere persistencia disponible y elimina las tablas de caché de
provider y documental, informando el conteo (`amatl-cli/src/main.rs:352-400`).
**No elimina telemetría.** Para una eliminación total autorizada, detén AMATL,
respalda sólo si la política lo permite y elimina el archivo SQLite configurado
junto con sus ficheros WAL/SHM usando controles del sistema operativo. El
repositorio no proporciona todavía un comando seguro de purga de telemetría;
esto es una limitación operativa explícita.
