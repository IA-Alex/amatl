# Identidad visual

Fuente única de verdad de la paleta de AMATL. Todo documento, mockup o
artefacto adicional que use color de marca (guías de diseño, propuestas de
logo, capturas anotadas, etc.) **debe** tomar los valores de acá — nunca
inventar un tono nuevo. Los tokens reales, ejecutables, viven en
`crates/amatl-ui/assets/styles.css`; este documento los explica y les da
contexto, no los reemplaza. Si en algún momento diverge de ese archivo, el
archivo tiene razón, no este documento.

## Origen

El nombre **AMATL** viene del náhuatl *āmatl*: papel amate, hecho
superponiendo y prensando tiras de corteza.

## El símbolo — caso aparte de la paleta

El ícono de marca (`crates/amatl-ui/assets/brand-icon.png`, servido en
`/brand-icon.png` y como favicon en `/favicon.png`) es un códice/libro
estilizado en un marrón/terracota fijo (`#7C572C` aprox.). **Es la única
pieza de la interfaz que no usa los tokens de esta página** — es
deliberado: el marrón es del símbolo únicamente, no forma parte del sistema
de color funcional. Botones, enlaces, foco de teclado y estados
(éxito/advertencia/error) siguen usando exclusivamente las tablas de abajo,
sin excepción. El símbolo no cambia entre tema claro y oscuro.

El wordmark "AMATL" que acompaña al símbolo se queda en la tipografía
monoespaciada ya documentada más abajo — el símbolo se reemplazó, la
tipografía del texto no.

## Paleta — tema oscuro (por defecto)

AMATL fue oscuro-only hasta que se agregó el interruptor de tema. Este bloque
es el `:root` base en `styles.css` — el que aplica sin ningún atributo ni
preferencia de sistema.

| Token | Hex | Uso |
|---|---|---|
| `--background` | `#111315` | Fondo de página |
| `--surface` | `#181B1F` | Tarjetas, paneles, superficies elevadas |
| `--border` | `#2A2F35` | Líneas divisorias, bordes de tarjeta |
| `--text` | `#E7E9EC` | Texto principal |
| `--secondary` | `#9DA5AE` | Texto secundario, etiquetas |
| `--muted` | `#6F7780` | Texto terciario, ayuda de campo |
| `--accent` | `#4F8CFF` | Enlaces, acento de marca |
| `--accent-strong` | `#3D6DC6` | Fondo de botón primario (texto blanco encima) |
| `--cyan` | `#48B8C7` | Acento secundario, foco de teclado |
| `--success` | `#4FAE72` | Estado positivo |
| `--warning` | `#D6A84B` | Estado de advertencia |
| `--error` | `#D95C5C` | Estado de error |
| `--error-on-tint` | `#E07C7C` | Texto de error sobre fondo `--error` translúcido |

## Paleta — tema claro (opt-in)

Se activa con el interruptor de tema (persiste en `localStorage`) o con
`prefers-color-scheme: light` del sistema si el operador nunca eligió. **No
son los tonos oscuros aclarados** — cada uno se recalculó oscureciéndolo
desde el mismo matiz, para seguir pasando WCAG 2 AA (4.5:1) como texto plano
sobre `--background`/`--surface`, igual que el tema oscuro.

| Token | Hex |
|---|---|
| `--background` | `#F5F7FA` |
| `--surface` | `#FFFFFF` |
| `--border` | `#D8DEE5` |
| `--text` | `#14181C` |
| `--secondary` | `#4B5560` |
| `--muted` | `#737D87` |
| `--accent` | `#2F6FE0` |
| `--accent-strong` | `#2757B8` |
| `--cyan` | `#157A8A` |
| `--success` | `#21804A` |
| `--warning` | `#92660F` |
| `--error` | `#B3342F` |
| `--error-on-tint` | `#7A231F` |

## Regla no negociable

**Ningún documento, artefacto o propuesta visual introduce un color
funcional que no esté en estas dos tablas.** Si algo necesita un tono que no
existe acá, la pregunta correcta es "¿cuál de los ya existentes cumple?", no
"¿qué hex se ve bien?". La única excepción reconocida es el marrón del
símbolo (arriba) — y es excepción precisamente porque está documentada acá
como tal, no porque cualquiera pueda sumar un color nuevo con el mismo
argumento.

## Tipografía

- **Texto general**: `Inter, system-ui, -apple-system, BlinkMacSystemFont,
  "Segoe UI", sans-serif`.
- **Wordmark y datos técnicos** (marca "AMATL", hashes, código): `"JetBrains
  Mono", ui-monospace, SFMono-Regular, Consolas, monospace`, negrita,
  `letter-spacing: 0.08em` para el wordmark.

Ninguna de las dos se sirve como webfont: dependen de lo que ya tenga
instalado el sistema del visitante, con esa pila de reemplazo. No agregar
`@font-face` ni CDN de fuentes — rompería la CSP (`script-src 'self'`,
`style-src 'self'`, sin orígenes externos).

## Cómo usarla en un documento nuevo

1. Copiar los valores de este archivo (o de `styles.css` directamente),
   nunca aproximarlos a ojo.
2. Si el documento distingue tema claro/oscuro, usar las dos tablas
   completas — no mezclar un tono oscuro con uno claro de otro token.
3. Si el documento es de un solo tema fijo (por ejemplo, una propuesta que
   solo se ve en oscuro), aclararlo explícitamente y usar únicamente la
   tabla de tema oscuro.
4. Actualizar este documento en el mismo cambio que actualice
   `styles.css`, no después.
