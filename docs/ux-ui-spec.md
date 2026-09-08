# Especificación UX/UI de AMATL

## Alcance

Auditoría y propuesta de diseño para la interfaz web de búsqueda y evidencia
en `crates/amatl-ui/assets/index.html`. Este documento no modifica la
implementación ni sustituye a `docs/identidad-visual.md`, que sigue siendo la
fuente única de verdad para los tokens de color.

## Usuario y objetivo

El usuario principal es un operador técnico que necesita encontrar documentos,
comparar fuentes y verificar la procedencia de fragmentos. Su tarea crítica es:

`escribir consulta → elegir modo → interpretar estado → abrir/guardar evidencia`.

El diseño debe privilegiar trazabilidad y lectura escaneable sobre densidad
decorativa. La acción primaria es **Buscar**; **Analizar evidencia** es una
acción avanzada; **Resumen con IA** sólo aparece disponible cuando el servicio
está configurado.

## Flujo principal

1. El foco inicial cae en `#query`.
2. El usuario escribe una consulta y pulsa **Buscar** (Enter también envía).
3. El estado anuncia progreso en `#status`; el control **Cancelar** aparece
   durante operaciones largas.
4. Los resultados se presentan como lista ordenada; cada tarjeta expone URL,
   título, fragmento y metadatos.
5. En modo profundo, cada documento añade estado, fragmentos verificables,
   acción **Guardar** y procedencia colapsada.
6. Paginación y paneles secundarios sólo se muestran cuando hay contenido.

## Jerarquía visual

- Nivel 1: marca, título de tarea y campo de consulta.
- Nivel 2: acciones de búsqueda y encabezado de resultados/estado.
- Nivel 3: títulos de documentos y fragmentos de evidencia.
- Nivel 4: metadatos, hashes, configuración y paneles administrativos.

Los paneles secundarios deben permanecer después de los resultados en el orden
del DOM y nunca competir visualmente con el campo de consulta.

## Sistema de diseño

- Grid: múltiplos de 8 px; separación base actual equivalente a `0.5rem`.
- Contenedor: máximo `72rem`; columna de lectura máxima `58rem`.
- Controles táctiles: mínimo 44×44 px (los botones actuales cumplen por altura;
  conservar esta garantía al añadir variantes).
- Radio: 8 px para superficies y controles; píldora sólo para estados/tags.
- Tipografía: Inter/system-ui para interfaz; JetBrains Mono para marca, URLs y
  hashes, según `docs/identidad-visual.md`.
- Estados: no depender sólo del color; combinar texto explícito (`success`,
  `partial`, `error`) con icono o etiqueta.

### Componentes y estados

| Componente | Estados requeridos | Regla |
|---|---|---|
| Botón primario | default, hover, focus-visible, pressed, disabled | `--accent-strong`, texto blanco |
| Botón secundario | default, hover, focus-visible, active, unavailable | Mantener clickeable si explica qué falta |
| Campo de consulta | vacío, focus, error, cargando | Mensaje asociado y no sólo placeholder |
| Tarjeta de resultado | default, enlace hover/focus | URL y título distinguibles |
| Evidencia | verified, range-only, failed, empty | Etiqueta textual + color funcional |
| Disclosure | cerrado, abierto, focus | Procedencia/configuración cerradas por defecto |

## Accesibilidad prevista

- Contraste AA usando exclusivamente los tokens de identidad existentes.
- `:focus-visible`, enlace para saltar a resultados y regiones `aria-live` ya
  presentes; conservarlos en futuras iteraciones.
- Orden de teclado: marca → tema → consulta → modos → filtros → resultados →
  paneles.
- Los estados de carga y error deben anunciarse con texto; respetar
  `prefers-reduced-motion`.
- Verificar que el botón no disponible explique el requisito mediante
  `aria-describedby`; `aria-disabled` no debe impedir la explicación.
- No truncar información crítica: el clamping móvil sólo debe aplicarse al
  snippet, nunca al título, estado o hash.

## Auditoría heurística inicial

Fortalezas observables: visibilidad del estado del sistema, control del usuario
(cancelar), reconocimiento sobre memorización (filtros etiquetados),
consistencia visual, disclosure para detalles técnicos y soporte responsive.

Riesgos a validar:

- Tres acciones juntas pueden aumentar la indecisión; mantener una sola acción
  primaria y explicar el beneficio de cada modo.
- El botón **Resumen con IA** puede parecer roto si el motivo de indisponibilidad
  no está visible antes del foco.
- Los paneles administrativos (clientes/tokens) compiten con la búsqueda si se
  muestran simultáneamente; deberían agruparse bajo un contexto de operación.
- En móvil, dos columnas de acciones reducen el ancho de las etiquetas; probar
  nombres largos y zoom al 200 %.

## Plan de validación

Este repositorio no contiene datos de pruebas con usuarios, por lo que no se
declaran métricas ficticias. Ejecutar antes de liberar:

1. Test moderado con 5 operadores: buscar, analizar evidencia, verificar un
   hash, guardar y cancelar una consulta.
2. Métricas: éxito por tarea, tiempo mediano, errores de modo, uso de teclado y
   comprensión del estado de verificación.
3. Auditoría automática con axe/Lighthouse y revisión manual de teclado,
   lector de pantalla, contraste, zoom 200 % y reduced motion.
4. Objetivos: ≥90 % de éxito en búsqueda, 0 bloqueos críticos de teclado,
   contraste WCAG 2.1 AA y CLS < 0.1; registrar LCP/INP en el entorno real.

## Entregables y límites

- Wireframe navegable de baja fidelidad: describir el flujo anterior o
  convertirlo a SVG/PNG desde la herramienta colaborativa elegida.
- Mockups: derivarlos directamente de los tokens de
  `docs/identidad-visual.md`; no introducir colores nuevos.
- Prototipo compartible: requiere un archivo/proyecto Figma proporcionado o
  acceso a una cuenta colaborativa; este repositorio por sí solo no contiene
  un enlace Figma.
- Assets actuales: `brand-icon.png`, `favicon.png` y las pilas tipográficas
  locales documentadas.
