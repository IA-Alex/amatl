# Auditoría conductual / inmersión — AMATL (UI web)

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
- **F-1 — Densidad del panel lateral en la primera visita.** Fuentes, servicio,
  historial, guardados y configuración de IA comparten columna
  (`index.html:181-204`). Para la tarea primaria (buscar y abrir) es ruido
  cognitivo temprano. Carga: ~6 regiones + `service-indicators` antes de que el
  usuario tenga un resultado. Recomendación: colapsar el panel completo tras un
  `<details>` o mostrarlo solo cuando `status !== "ok"` o bajo demanda.
- **F-2 — Estados de degradación potencialmente alarmantes sin acción.**
  `partialResults`, `partialEvidence`, `unavailable` (`i18n.js:59-65,94`) son
  honestos, pero no siempre dicen **qué hacer**. "algunas fuentes no
  respondieron" no ofrece un camino (p.ej. "probar otra fuente"). Fricción de
  incertidumbre sin salida.
- **F-3 — El error de proveedor y el error de red comparten tono.** `error*`
  (`i18n.js:73-81`) son claros y distintos, lo cual es bueno; pero no siempre hay
  un CTA de recuperación adyacente (p.ej. reintentar / cambiar fuente).
- **F-4 — `AI summary` visible pero `is-unavailable`.** Un control deshabilitado
  persistente añade una decisión fantasma ("¿por qué no puedo?"). El `answer-hint`
  (`index.html:108`) existe, pero nace `hidden`; conviene mostrarlo junto al
  botón deshabilitado para cerrar el bucle.

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

- **R-1 (F-1) — Separar las dos experiencias.**
  - *Problema:* consola de operador compite con la tarea de búsqueda.
  - *Mecanismo:* carga cognitiva / atención selectiva.
  - *Evidencia:* `index.html:181-204` (panel siempre presente).
  - *Consecuencia:* menor comprensión inicial y más abandono pre-resultado.
  - *Cambio:* envolver el panel en `<details>` (colapsado) y auto-expandirlo solo
    si `status !== "ok"`.
  - *Efecto esperado:* primera fijación en el campo de consulta.
  - *Riesgo:* operadores que quieren ver el estado de inmediato (mitigar con el
    `summary` mostrando un badge de estado).
  - *Validación:* scroll-depth al panel en primera visita; tiempo hasta primera
    consulta.

- **R-2 (F-2/F-3) — Microcopía con salida.**
  - *Problema:* estados degradados sin acción.
  - *Mecanismo:* reducción de incertidumbre / autoeficacia.
  - *Evidencia:* `i18n.js:59-65` (sin verbo de acción).
  - *Cambio:* añadir a `partialResults`/`partialEvidence` un enlace o botón
    "Probar otra fuente" que abra el selector de fuentes.
  - *Efecto esperado:* menos abandono ante respuesta parcial.
  - *Riesgo:* ruido si la degradación es benigna (mostrarlo solo cuando hay >1
    fuente configurada).
  - *Validación:* tasa de recuperación tras `partial_*`.

- **R-3 (F-4) — Cerrar el bucle del botón deshabilitado.**
  - *Problema:* `AI summary` deshabilitado sin explicación visible.
  - *Evidencia:* `index.html:62` (`is-unavailable`) + `answer-hint` `hidden:108`.
  - *Cambio:* mostrar `answer-hint` (motivo + cómo habilitarlo) cuando el botón
    esté deshabilitado.
  - *Mecanismo:* coherencia expectativa↔capacidad.
  - *Riesgo:* bajo. *Validación:* clics en zona del botón y apoyo contextual.

- **R-4 — Reservar "Próxima acción" tras abrir.** Añadir, tras el primer
  resultado abierto, una sugerencia discreta (p.ej. "Guardar este documento" o
  "Analizar evidencia de esta consulta"). *Mecanismo:* efecto goal-gradient;
  *riesgo:* empujar acciones de coste superior sin pedirlo → mantenerlo como
  sugerencia no intrusiva. *Validación:* uso de Deep/save tras una apertura.

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
  Las hipótesis están marcadas como tales.

### Quality Gate
- [x] Objetivo conductual explícito · [x] Contexto de usuario entendido
- [x] Mecanismo psicológico identificado · [x] Recomendación con implementación concreta
- [x] Evidencia e hipótesis separadas · [x] Inmersión evaluada en conjunto
- [x] Carga cognitiva considerada · [x] Fricción clasificada
- [x] Autonomía preservada · [x] Las promesas coinciden con el producto
- [x] Impacto medible · [x] Efectos no deseados considerados
