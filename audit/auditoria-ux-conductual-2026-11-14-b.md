# Auditoría conductual / inmersión — AMATL (UI web)

> **Corrección (2026-11-15).** Verificado contra `app.js` en una segunda pasada:
> **R-3 era una falsa alarma** (el botón `AI summary` ya muestra el motivo antes
> del clic; el informe se basó en el `hidden` del HTML estático sin trazar el JS) y
> **la evidencia de R-1 era incorrecta** (el panel lateral nace `hidden`, no
> "siempre presente"). R-2 y R-4 siempre fueron recomendaciones de producto, no
> defectos. Ver la sección 7 para el detalle. Causa raíz de ambos errores: análisis
> solo estático, sin `browser_e2e`.

## 1. Experience Objective
La UI busca producir **orientación y confianza operativa**: que una persona
formule una consulta y obtenga, sin fricción, *resultados que pueda abrir y
verificar*. El flujo declarado por el propio producto es **buscar → revisar →
abrir** (`core.md`). No busca engagement prolongado ni retención; busca una tarea
completada con evidencia.

Objetivo conductual medible: **tasa de consultas que terminan en una apertura de
resultado o en una decisión informada** ("esta búsqueda no sirve, cambio de
fuente"), no número de clics.

## 2. Psychological Model
Mecanismos relevantes **observados** por el diseño:
- **Reducción de incertidumbre** vía `role="status" aria-live="polite"` en
  `#status` y `#service-summary` (feedback continuo).
- **Jerarquía de compromiso**: "Search" (primario, sólido) vs "Analizar
  evidencia" (secundario) vs "AI summary" (terciario + `is-unavailable`)
  (`index.html:60-62`). Es una escala de coste creciente explícita.
- **Divulgación progresiva**: filtros y acceso al servidor viven en `<details>`
  colapsados (`index.html:78,145`); el toggle de estratega nace `hidden`
  (`index.html:70`).
- **Saliencia de estado**: indicadores de fuente y servicio en panel lateral
  (`index.html:181-198`).
- **Ausencia deliberada de dark patterns**: sin contadores de urgencia, sin
  "X personas buscaron esto". Coherente con la ética del producto.

Hipótesis (no observada directamente): el usuario típico llega con una pregunta
concreta y con un modelo mental de "motor de búsqueda", no de "panel de
observabilidad"; la densidad del panel lateral puede competir con esa expectativa.

## 3. Current Behavioral Journey
```
Entry  → formulario arriba, heading "Busquemos" (cálido, baja amenaza)
   ↓
Orientation → tres botones con ayuda (#search-mode-help) explican la diferencia
   ↓
Value perception → resultados rankeados; snippet + dominio + fecha
   ↓
Trust → panel de fuentes/servicio; estados "degraded"/"unavailable" explícitos
   ↓
Action → abrir resultado (enlace externo)
   ↓
Experience → paginación server-side; historial y guardados (si hay storage)
   ↓
Continuation → cambiar de fuente/consulta; guardar documento
```

## 4. Friction Analysis

### Fricción útil (conservar)
- **Ayuda de modo de búsqueda** (`#search-mode-help`, `i18n.js` `searchModeHelp`):
  desambigua tres acciones de coste muy distinto. Necesaria para decisión informada.
- **Token en campo de solo-memoria** (`index.html:149-150`,
  `tokenHelp`): fricción de seguridad legítima; se explica y no se persiste.
- **Confirmación de umbral** (token mínimo 32, `errorTokenTooShort`): evita
  errores costosos.

### Fricción innecesaria (candidata)
- **F-1 — Densidad del panel lateral [CORREGIDO].** Evidencia original
  "`index.html:181-204`, panel siempre presente" — **incorrecto**: las secciones
  nacen `hidden` (`index.html:180,186,202`) y se revelan bajo demanda. Cuando sí
  se muestran, varias regiones coexisten en la misma columna. Se degrada a
  observación menor, no defecto.
- **F-4 — Botón `AI summary` [RETIRADO].** El informe afirmó que quedaba
  "deshabilitado sin explicación". **Falso**: `app.js:1246-1268` lo mantiene
  clicable y muestra `answer-hint` antes del clic con `aria-describedby` + `title`.
  Era una falsa alarma.
- **F-2 — Estados de degradación potencialmente alarmantes sin acción.**
  `partialResults`, `partialEvidence`, `unavailable` (`i18n.js:59-65,94`) son
  honestos, pero no siempre dicen **qué hacer**. "algunas fuentes no
  respondieron" no ofrece un camino (p.ej. "probar otra fuente"). Fricción de
  incertidumbre sin salida.
- **F-3 — El error de proveedor y el error de red comparten tono.** `error*`
  (`i18n.js:73-81`) son claros y distintos, lo cual es bueno; pero no siempre hay
  un CTA de recuperación adyacente (p.ej. reintentar / cambiar fuente).

## 5. Immersion Analysis

- **Presence (voz consistente):** fuerte. Todo el texto visible vive en
  `i18n.js` (`i18n.js:8-9` lo declara como punto único), con paridad `es`/`en`.
  No hay copy hardcodeado en `app.js`. Esto sostiene una voz estable.
- **Entry:** "Busquemos" (es) / heading corto, formulario dominante, cero
  jerga técnica en la superficie primaria. Buen umbral de entrada.
- **Continuity:** los mensajes de estado (`searching → resultados/parcial/vacío`)
  construyen continuidad con `aria-live`; el usuario nunca queda "colgado".
- **Rotura de inmersión:** el panel lateral introduce un **cambio de registro**
  (de "búsqueda conversacional" a "consola de operador") en la misma pantalla.
  Es la discontinuidad principal: dos experiencias con perfiles psicológicos
  distintos conviviendo sin separación visual fuerte.
- **Exit:** hay cierre razonable (paginación, historial, guardados) pero sin
  "próxima acción significativa" explícita tras abrir un resultado. No hay
  ansiedad manufacturada por salir (correcto).

## 6. Content Analysis
(Capas sobre microcopía clave; evidencia en `i18n.js`.)

| Texto | Semántica | Emoción | Conducta que facilita | Riesgo de mala interpretación |
|---|---|---|---|---|
| `searchModeHelp` | Explica coste de 3 modos | Claridad | Elegir el modo correcto | Baja: es preciso |
| `partialResults` ("resultados; algunas fuentes no respondieron") | Honestidad | Leve desconfianza | Aceptar resultados con cautela | Media: no dice *qué hacer* |
| `unavailable` ("No disponible") | Estado | Neutra/frustración | — | Media: sin causa ni salida aquí |
| `errorProviderUnavailable` ("No hay una fuente utilizable…") | Diagnóstico | Frustración | Saber que no es su culpa | Baja: claro |
| `errorStorageUnavailable` | Diagnóstico | Neutra | Entender por qué no hay historial | Baja |

Fortaleza: **los mensajes no prometen lo que el producto no hace** (coherente con
la doctrina). Debilidad: faltan microcopias **orientadas a la acción** junto a los
estados degradados.

## 7. UX Recommendations

- **R-3 (F-4) — [RETIRADO: falsa alarma, NO es un defecto.]** El informe afirmó
  que el botón `AI summary` quedaba "deshabilitado sin explicación visible".
  Verificado contra el código: **es incorrecto**. `app.js:1246-1268` mantiene el
  botón siempre visible y clicable (nunca `disabled` real), y cuando
  `answerStatus.available === false` **muestra `answer-hint` antes del clic**
  (`answerHint.hidden = false`, línea 1265) y lo enlaza con el control vía
  `aria-describedby` + `title` (líneas 1266-1267). El comentario en
  `app.js:1251-1256` documenta que esto ya se corrigió antes. La premisa original
  se basó en el atributo `hidden` del HTML estático (`index.html:108`) sin trazar
  el JS que lo revela. **Causa del error de auditoría:** análisis solo estático,
  sin `browser_e2e` (limitación ya declarada en la sección 10).

- **R-1 (F-1) — [MATIZADO: evidencia corregida; mejora opcional, no defecto.]**
  *Evidencia original:* "`index.html:181-204` (panel siempre presente)" —
  **incorrecto**: las secciones del panel lateral nacen `hidden` y se revelan bajo
  demanda (`index.html:180,186,202` + `refreshPanels()` en `app.js`), no están
  siempre visibles.
  - *Problema:* cuando el panel sí se muestra, coexisten dos registros en pantalla.
  - *Mecanismo:* carga cognitiva / atención selectiva.
  - *Consecuencia:* posible menor comprensión cuando el panel aparece.
  - *Cambio:* envolver el panel en `<details>` (colapsado) y auto-expandirlo solo
    si `status !== "ok"`.
  - *Efecto esperado:* primera fijación en el campo de consulta.
  - *Riesgo:* operadores que quieren ver el estado de inmediato (mitigar con el
    `summary` mostrando un badge de estado).
  - *Validación:* scroll-depth al panel en primera visita; tiempo hasta primera
    consulta.

- **R-2 (F-2/F-3) — [RECOMENDACIÓN de producto, no defecto.]** Microcopía con
  salida. El estado de resultados parciales es honesto y deliberado (ver
  `app.js:776-788`: el estado vacío es neutro y "la nota bajo la lista lleva el
  siguiente paso"). Añadir un CTA es una **decisión de producto**, no la
  corrección de un fallo.
  - *Problema:* estados degradados sin acción adyacente.
  - *Mecanismo:* reducción de incertidumbre / autoeficacia.
  - *Evidencia:* `i18n.js:59-65` (sin verbo de acción).
  - *Cambio propuesto:* añadir a `partialResults`/`partialEvidence` un enlace o
    botón "Probar otra fuente" que abra el selector de fuentes.
  - *Efecto esperado:* menos abandono ante respuesta parcial.
  - *Riesgo:* ruido si la degradación es benigna (mostrarlo solo cuando hay >1
    fuente configurada).
  - *Validación:* tasa de recuperación tras `partial_*`.

- **R-4 — [RECOMENDACIÓN de producto, no defecto.]** Reservar "Próxima acción"
  tras abrir. Añadir, tras el primer resultado abierto, una sugerencia discreta
  (p.ej. "Guardar este documento" o "Analizar evidencia de esta consulta").
  *Mecanismo:* efecto goal-gradient; *riesgo:* empujar acciones de coste superior
  sin pedirlo → mantenerlo como sugerencia no intrusiva. *Validación:* uso de
  Deep/save tras una apertura.

## 8. Implementation Guidance
- **Copy/CTA:** añadir claves nuevas en `i18n.js` en **ambos** catálogos (paridad
  verificada por tests de servidor — `docs/testing.md`), p.ej. `partialRecoverCTA`.
- **Layout/jerarquía:** convertir el panel lateral en `<details id="service-panel">`
  colapsado; mantener `#status` como canal de estado primario.
- **Interacción:** al recibir `status: partial_*`, revelar un botón "Cambiar
  fuente" que abra el `<details id="filters">` ya existente (`index.html:78`).
- **Accesibilidad:** conservar `aria-live="polite"`; cualquier CTA nuevo debe ser
  alcanzable por teclado (hay E2E de flujo solo-teclado, `browser_e2e.rs`).
- **Onboarding:** el `#search-mode-help` ya hace onboarding de modos; no añadir tour.
- **Animación:** respetar `prefers-reduced-motion` (ya cubierto por el contrato de
  layout).

## 9. Measurement
- **Métricas conductuales:** tiempo hasta primera consulta; tasa de apertura de
  resultado; tasa de recuperación tras `partial_*`; uso de "Cambiar fuente";
  abandono antes del primer resultado.
- **Guardrails:** no premiar más clics como señal de éxito; vigilar que
  R-4 no aumente Deep/save no deseados; vigilar que colapsar el panel (R-1) no
  oculte degradaciones reales a operadores.
- **Interpretación en contexto:** sesiones más largas **no** son mejor señal aquí.

## 10. Risks
- **Riesgo de falsa simplicidad:** colapsar el panel puede ocultar que hay
  degradación. Mitigación: badge en el `summary` del panel.
- **Riesgo de sobre-recomendación:** R-4 puede empujar acciones costosas. Se
  mantiene como sugerencia no intrusiva.
- **Riesgo de i18n:** cualquier copy nueva debe mantener paridad `es`/`en` o
  rompe tests de paridad.
- **Sesgo de este informe:** basado en análisis estático de assets y API real, no
  en observación de usuarios ni en UI renderizada (`browser_e2e` no ejecutado).
  Las hipótesis están marcadas como tales. **Este sesgo produjo dos hallazgos
  falsos** (R-3 y la evidencia de R-1) al no trazar el JS que revela elementos
  `hidden`; ambos están corregidos arriba.

### Quality Gate
- [x] Objetivo conductual explícito · [x] Contexto de usuario entendido
- [x] Mecanismo psicológico identificado · [x] Recomendación con implementación concreta
- [~] Evidencia e hipótesis separadas — **falló en la 1.ª pasada** (R-3 declarada
  defecto con evidencia solo estática); **corregido el 2026-11-15** contra el código
- [x] Inmersión evaluada en conjunto
- [x] Carga cognitiva considerada · [x] Fricción clasificada
- [x] Autonomía preservada · [x] Las promesas coinciden con el producto
- [x] Impacto medible · [x] Efectos no deseados considerados
