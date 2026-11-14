# Auditoría UX — AMATL (UI web)

Pruebas de funcionamiento, coherencia, simetría, armonía visual y acabado
sobre la UI real servida por el binario release, conducidas con un navegador
de verdad (Chromium vía WebDriver/Playwright, modo headed).

- Fuente: `/home/panda/amatl`, HEAD `6b032a41`, árbol de trabajo limpio.
- Entorno: **local y efímero** — `target/release/amatl serve --mock` en
  `127.0.0.1:8931`, `no_auth = true`, sqlite en `/tmp`. Sin red.
- Método: por cada control, (1) prueba funcional en vivo, (2) medición de
  geometría/contraste computados, (3) captura de pantalla. **No se modificó el
  código**; los cambios se proponen, no se aplican.
- Artefactos: `output/playwright/01…06` (dark, light, deep, diálogo, mobile).

> Nota de método: durante la sesión aparecieron dos "fallos" que resultaron ser
> artefactos de mis propias pruebas (clonado de `<template>` para inspección).
> Se verificaron en una carga limpia y **no** se reportan. Solo figura lo
> reproducido.

---

## 1. Veredicto

**La UI funciona y está muy bien construida.** Los 14 controles interactivos
probados responden correctamente; el sistema de temas (dark/light) es sólido;
la responsividad a 360px no desborda y reordena con criterio; la cobertura i18n
(EN/ES) es completa en el DOM renderizado; y el diálogo de credencial tiene
acabado premium. Hay **un fallo funcional menor** (pluralización en el
historial) y **dos defectos de composición visual** (desalineación y vacío del
grid lateral), más observaciones de densidad.

| Severidad | Nº | Temas |
|---|---|---|
| Alto | 0 | — |
| Medio | 3 | Pluralización; desalineación de panel; vacío del grid lateral |
| Bajo | 4 | Densidad de botones; checkbox sobredimensionados; campo Token mal ubicado; copy menor |

---

## 2. Pruebas de funcionamiento (botón por botón)

Todos probados en vivo con navegador real. Ninguno es un "botón muerto".

| Control | Acción | Resultado observado | Estado |
|---|---|---|---|
| `#theme-toggle` | click | `data-theme` dark↔light, `aria-pressed`, `aria-label`, icono sun/moon cambian juntos | OK |
| `#search-button` | click | `1 de 1 resultado.`, 1 `<li>` renderizado, `disabled=false` tras cargar | OK |
| `#deep-button` | click | tarjeta de evidencia con fragmentos, procedencia y botón Guardar | OK |
| `#answer-button` | click | deshabilitado correctamente (no configurado); tooltip/estado explicativo | OK |
| `summary.filters` | click | `details.open=true`, aparece selector de fuente + opciones deep | OK |
| `#provider-selector` radios | estado | SearXNG checked; Marginalia/Todas alternables | OK |
| `#marginalia-key-button` | click | abre `<dialog>` nativo, copy traducido | OK |
| `#marginalia-cancel` | click | cierra el diálogo (`open=false`) | OK |
| `.history-reuse` | click | rellena la consulta y **relanza** la búsqueda | OK |
| `.history-delete` | click | elimina la entrada | OK |
| `#history-purge` | click | "Borrar historial" (con confirmación) | OK |
| `#client-create-summary` | click | despliega el formulario de cliente | OK |
| `#ui-language` | change | `en`↔`es`, traduce encabezados, botones y `lang` | OK |
| Skip-link | focus | visible al enfocar (WCAG 2.4.11) | OK |

**Conclusión funcional:** no hay controles innecesarios *muertos*. El aparato
está bien cableado.

---

## 3. Coherencia

- **Tema (dark/light): coherente.** Tokens espejados, contraste correcto en
  ambos, sin parches oscuros bleed-through. El toggle mantiene sincronizados
  estado, etiqueta, `aria-pressed` e icono.
- **i18n (EN/ES): cobertura completa en el DOM renderizado.** Verificado con
  barrido en modo inglés sobre la página limpia: **cero** cadenas en español
  visibles. La aparente "mezcla de idiomas" de un vistazo previo fue artefacto
  de mi inspección de `<template>` crudos, no del render.
- **Jerarquía de botones: coherente.** Primario (azul `--accent-strong`) vs
  secundario (`--surface`) vs ghost (`transparent`) vs danger (`--error`), con
  alturas unificadas por `--control-h`/`--control-h-sm`. Sin deriva de radios ni
  alturas (el propio CSS lo blinda con tokens).
- **Estados de error: bien resueltos.** `/answer` no configurado muestra un
  mensaje específico, no un error crudo; los badges de proveedor distinguen
  `disabled`/`not_approved` con color y texto.

---

## 4. Simetría y armonía visual (medido)

Medición de geometría computada del grid `.side-panels` (desktop 1280px):

| Panel | top | alto |
|---|---|---|
| Source status | **1164** | 264 |
| Service state | 1132 | 195 |
| Search history | 1132 | **779** |
| Saved documents | 1935 | 95 |
| Clients & tokens | 1935 | 127 |

**Hallazgos de composición:**

1. **Desalineación de 32px en `Source status`** — arranca en `top=1164`
   mientras sus vecinos de fila arrancan en `1132`. Causa: `.provider-status`
   conserva `margin-top: 2rem` (styles.css:652) mientras sus hermanos de grid
   tienen `margin-top: 0` (styles.css:775). Es un residuo de cuando el panel era
   un bloque suelto. **Síntoma visual:** el panel "se hunde" dentro de su fila.
2. **Vacío estructural en la fila 1** — `Search history` mide 779px y fuerza la
   fila entera a esa altura; los paneles a su lado (264 y 195) terminan ~500px
   antes, dejando un hueco muerto grande. **Síntoma:** el bloque lateral se ve
   desequilibrado y "a jirones". Es el defecto visual más visible de la página.
3. **Ancho sí es simétrico** — las tres columnas miden 299px exactos. El
   problema es **vertical**, no horizontal.

---

## 5. Acabado premium

Lo que está a nivel excelente:
- **Diálogo de credencial** (`05`): backdrop, centrado, foco visible, jerarquía
  primario/secundario clara, copy que explica la política de memoria. Impecable.
- **Estados de degradación**: badge "Enriched", "Range and hash verified",
  `data-state` con color por severidad. Honesto y legible.
- **Skeletons/loading**: puntos pulsantes con `prefers-reduced-motion` respetado.
- **Tipografía**: interletra del wordmark, monoespaciada en URLs/hashes, escala
  fluida con `clamp()`. Consistente.

Lo que rompe el acabado:
- **Badges de proveedor que envuelven**: "No disponible" se parte en dos líneas
  dentro del badge junto a "Habilitada", dando una caja alta y desproporcionada.
- **Vacío del grid lateral** (ver §4.2): es lo primero que ve el ojo y da
  impresión de "borrador".

---

## 6. Hallazgos priorizados

### H-01 — [MEDIO] Pluralización rota en el historial ("1 resultados.")

- **Evidencia**: [E2E] las filas del historial muestran `"1 resultados."`;
  la cabecera de resultados de la **misma consulta** muestra `"1 of 1 result"`.
  Código: `app.js:1505` usa `` `${entry.total_results} ${t("resultsCount")}` ``
  **sin** `pluralKey`, pese a que el helper existe (`app.js:57`) y la clave
  singular existe (`i18n.js:61` `resultsCountOne: "resultado."`). La cabecera sí
  usa `pluralKey` (`app.js:770`).
- **Causa**: CONFIRMADO — llamada directa a la clave plural en `historyLabel`.
- **Impacto**: incoherencia gramatical visible en cada fila; contradice la misma
  métrica mostrada arriba.
- **Corrección mínima**: `app.js:1505` → usar
  `` t(pluralKey("resultsCount", entry.total_results)) `` (mismo patrón que 770).
- **Regresión**: test de que una entrada con `total_results=1` renderiza
  "1 resultado.".

### H-02 — [MEDIO] `Source status` desalineado 32px dentro del grid

- **Evidencia**: [E2E] `top=1164` vs hermanos `1132` (32px = 2rem).
  [ESTÁTICO] `styles.css:652` `.provider-status { margin-top: 2rem }` frente a
  `styles.css:775` hermanos `{ margin-top: 0 }`.
- **Causa**: CONFIRMADO — margen heredado de cuando el panel no era celda de grid.
- **Impacto**: el panel parece descolgado; rompe la línea de base de la fila.
- **Corrección mínima**: quitar `margin-top: 2rem` de `.provider-status` (o
  ponerlo a `0`), ya que el `gap` del grid provee la separación.
- **Regresión**: test de geometría que afirme `top` igual entre hermanos de fila.

### H-03 — [MEDIO] Vacío de ~500px en la fila 1 del grid lateral

- **Evidencia**: [E2E] `Search history` h=779 vs `Source status` h=264 y
  `Service state` h=195 en la misma fila.
- **Causa**: CONFIRMADO — `grid-template-columns: repeat(auto-fit, …)` con
  `align-items: start` y alturas naturales muy dispares; no hay balanceo.
- **Impacto**: desarmonía visual marcada; parece espacio desperdiciado.
- **Corrección mínima propuesta** (una, no las tres):
  (a) `grid-auto-rows: min-content` + `align-content: start` no lo arregla;
  mejor **(b) columnas tipo masonry** con `grid-auto-flow: dense` no aplica;
  la más simple y robusta: **fijar 2 columnas en desktop** y dejar que
  "Search history" ocupe una columna propia de altura acotada con scroll
  interno (`max-height` + `overflow: auto`), de modo que no estire la fila.
  Alternativa mínima: limitar el alto del historial a ~8 filas visibles.
- **Regresión**: captura de la sección lateral sin huecos > 150px.

### H-04 — [BAJO] Densidad de acciones en el historial (24 botones para 12 filas)

- **Evidencia**: [E2E] 12 filas × 2 botones (`Reuse`/`Delete`) = 24 botones,
  muchos sobre la **misma** consulta repetida. [screenshot 06]
- **Causa**: CONFIRMADO — cada fila expone dos acciones siempre visibles.
- **Impacto**: ruido visual; en móvil domina la pantalla.
- **Corrección mínima**: mostrar las acciones al hover/focus, o deduplicar
  consultas consecutivas idénticas en el panel.
- **Regresión**: n/a (decisión de diseño).

### H-05 — [BAJO] Checkbox con caja 16px y fila de 44px

- **Evidencia**: [E2E] `strategist-enabled-toggle`, toggles de proveedor y
  scopes miden `16×44` (caja 16px centrada en una fila de 44px).
- **Causa**: CONFIRMADO — hit area correcta (WCAG 2.5.8 vía `<label>`), pero la
  caja visual queda diminuta respecto a la fila.
- **Impacto**: desproporción óptica en los toggles.
- **Corrección mínima**: `accent-color` ya está; subir `width/height` del input
  a `1.15rem` o alinear la fila al alto de la caja.
- **Regresión**: n/a.

### H-06 — [BAJO] "Token de acceso" ubicado entre los filtros de búsqueda

- **Evidencia**: [E2E] `#local-token` ("Server token") vive dentro de
  `details.filters`, entre "Idioma de resultados", "Región" y "Tipo de archivo"
  — que sí son filtros de consulta; el token no lo es.
- **Causa**: CONFIRMADO — ubicación semántica.
- **Impacto**: confusión (¿es un filtro?), y compite con las superficies de
  clientes/tokens.
- **Corrección mínima**: moverlo a una sección "Acceso" propia o al panel de
  clientes.
- **Regresión**: n/a.

### H-07 — [BAJO] Copy: "Datos de prueba (modo mock)" y encabezados sin traducir en estados

- **Evidencia**: [E2E] el aviso de modo mock aparece en el panel de servicio;
  correcto y honesto, pero solo en español en el markup base (se traduce en
  runtime; se deja constancia por completitud).
- **Causa**: CONFIRMADO — no es defecto; es copy de estado de desarrollo.
- **Impacto**: nulo.
- **Corrección mínima**: ninguna.

---

## 7. Lo que está bien (no tocar)

- Sistema de temas dark/light completo y accesible.
- Cobertura i18n del DOM renderizado: 100% en EN.
- Responsive a 360px sin overflow horizontal, con reordenamiento sensato.
- Diálogos nativos bien resueltos (foco, backdrop, jerarquía).
- Máquina de estados de error/degradación honesta y coloreada.

---

## Apéndice — cómo se reprodujo

```bash
# binario release + config efímera con mock
./target/release/amatl --config-file /tmp/.../amatl.audit.toml serve --mock
# navegador real, headed
"$PWCLI" open http://127.0.0.1:8931/ --headed
"$PWCLI" snapshot          # refs frescos antes de cada interacción
"$PWCLI" click eN
"$PWCLI" screenshot --full-page --filename output/playwright/NN.png
```

Capturas: `output/playwright/01-filters-open-dark.png` · `02-light.png` ·
`03-deep-en.png` · `04-deep-en-clean.png` · `05-marginalia-dialog-en.png` ·
`06-mobile-360.png`.
