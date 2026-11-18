# Auditoría AMATL — 2026-11-13

Auditoría estática + comandos, nivel **Completo** (frontend + backend + análisis detallado).
Fuente: `/home/panda/amatl`, rama `chore/start-sh-token-bootstrap`, HEAD `45559cb2`,
árbol de trabajo **limpio** (sin cambios locales) al iniciar.

---

## 1. Resumen ejecutivo

El proyecto está **muy por encima de la media**: arquitectura en capas coherente, 695 tests
en verde, clippy/rustdoc limpios, defensas SSRF/authc/authz trazables a la petición, y una
disciplina de contratos documental excepcional. Pero el estado **no es "verde"** hoy:

- El **gate de calidad local falla en 2 de 3 puertas**: `cargo fmt --check` (4 diffs) y
  `cargo audit`/`cargo deny check` (**5 vulnerabilidades**), y CI ejecuta ambas en cada push.
- Hay un **bypass de SSRF real y verificado** (`::10.0.0.1` etc. tratados como públicos).
- La documentación de seguridad afirma "`cargo audit` is green" cuando hoy no lo está.

| Severidad | Nº |
|---|---|
| Crítico | 0 |
| Alto | 3 |
| Medio | 3 |
| Bajo | 4 |

**Top 3 riesgos**: (1) SSRF por IPv6 compatible (`SEC-001`), (2) CI rojo por advisories sin
remediar (`CHAIN-001`), (3) `cargo fmt --check` rojo commiteado (`BUILD-001`).

---

## 2. Alcance y método

- **Revisado**: `crates/amatl-core` (SSRF, auth, storage, config), `crates/amatl-server`
  (middleware de seguridad, rutas, MCP), `crates/amatl-ui` (JS vanilla, CSP, i18n),
  `Cargo.toml`/`Cargo.lock`, `deny.toml`, `.github/workflows/`, docs de seguridad.
- **No revisado**: `experiments/*` (excluidos a propósito), runtime real en navegador
  (sin WebDriver vivo), macOS/Windows, `release.yml`/`publish-*.yml` en ejecución,
  integración real de Trafilatura y Chromium.
- **Modo**: estático + comandos. **Sin modificar** ningún fichero del proyecto.
- **Comandos ejecutados** (desde la raíz, `--locked`):
  `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features`,
  `cargo test --workspace` (×5), `cargo doc` (con y sin `-D warnings`),
  `cargo audit`, `cargo deny check`, `cargo run -p amatl-cli -- benchmark ranking-v2 --json`,
  `git log/status/diff`. Programa Rust compilado ad-hoc para refutar/confirmar `is_public_ip`.

---

## 3. Tabla de hallazgos

| ID | Sev. | Tipo | Ubicación | Resumen | Esf. | Certeza |
|---|---|---|---|---|---|---|
| SEC-001 | Alto | Riesgo | `crates/amatl-core/src/security.rs:91-102` | SSRF: IPv6 IPv4-compatible (`::10.0.0.1`, `::169.254.169.254`) tratadas como públicas | S | VERIFICADO |
| CHAIN-001 | Alto | Riesgo | `Cargo.lock` (`rmcp 3.1.2`, `rustix 1.1.4`, `quinn-udp 0.5.15`, `libsqlite3-sys 0.30.1`) | `cargo audit` y `cargo deny check` fallan: 5 advisories + 1 unsound. CI rojo | M | VERIFICADO |
| BUILD-001 | Alto | Bug | `crates/amatl-cli/tests/cli.rs`, `crates/amatl-core/src/service.rs` | `cargo fmt --check` falla (4 diffs) en código commiteado; CI rojo | S | VERIFICADO |
| DOC-001 | Medio | Deuda | `docs/security/supply-chain.md:96,116-117` | Afirma "`cargo audit` is green" y "`cargo deny check` is clean"; hoy es falso | S | VERIFICADO |
| TEST-001 | Medio | Riesgo | suite `cargo test --workspace` | 1 fallo intermitente observado (695→1 fallo→695) no caracterizado | M | VERIFICADO (síntoma) / causa NO VERIFICADA |
| CHAIN-002 | Medio | Deuda | `crates/amatl-core/tests/supply_chain_contract.rs:28-75` | El floor test offline no cubre ninguna de las 5 advisories hoy abiertas | S | VERIFICADO |
| FE-001 | Bajo | Bug | `crates/amatl-ui/assets/app.js:821` | `decodeURIComponent` sin guardia aborta el bootstrap con `/?q=%` | S | VERIFICADO |
| FE-002 | Bajo | Mejora | `crates/amatl-ui/assets/app.js:1055` | `provider.status` sin validar se interpola como token de `className` | S | VERIFICADO |
| BE-001 | Bajo | Mejora | `crates/amatl-core/src/robots.rs:285-287` | La caché de robots hace `clear()` al llenarse (cache-thrash), no evict-uno | S | VERIFICADO |
| DOC-002 | Bajo | Deuda | `docs/security/hallazgos-2026-10-26.md:137,183`; `amatl_revision_critica.md` H5 | Citas a `docs/api/openapi.yaml` y a `chacha20 0.10.1`, ambos ya inexistentes/superados | S | VERIFICADO |

---

## 4. Detalle de hallazgos Alto

### SEC-001 — SSRF: IPv6 IPv4-compatible evita el filtro de direcciones

**Ubicación**: `crates/amatl-core/src/security.rs:91-102` (`public_v6`), consumido por
`is_public_ip` (`:69`) y `validate_resolved_addresses` (`:32`).

**Evidencia**:
```rust
// security.rs:91-102
fn public_v6(address: Ipv6Addr) -> bool {
    if let Some(mapped) = address.to_ipv4_mapped() {   // solo captura ::ffff:a.b.c.d
        return public_v4(mapped);
    }
    let segments = address.segments();
    !(address.is_loopback() || address.is_unspecified() || address.is_multicast()
        || (segments[0] & 0xfe00) == 0xfc00        // fc00::/7
        || (segments[0] & 0xffc0) == 0xfe80        // fe80::/10
        || (segments[0] == 0x2001 && segments[1] == 0x0db8))  // 2001:db8::/32
}
```
`to_ipv4_mapped()` **solo** reconoce la forma *mapped* `::ffff:x.x.x.x`. La forma
*IPv4-compatible* (deprecada, RFC 4291 §2.5.5.1) `::x.x.x.x` **no** la reconoce, y ninguno
de los predicados la cubre. Programa de prueba compilado contra la lógica real de la crate:
```
::127.0.0.1        to_ipv4=Some(127.0.0.1)      allowed(public)=true
::10.0.0.1         to_ipv4=Some(10.0.0.1)       allowed(public)=true
::192.168.1.1      to_ipv4=Some(192.168.1.1)    allowed(public)=true
::169.254.169.254  to_ipv4=Some(169.254.169.254) allowed(public)=true
::172.16.0.1       to_ipv4=Some(172.16.0.1)     allowed(public)=true
```

**Refutación intentada**: busqué cualquier guardia posterior entre
`validate_resolved_addresses` y el connect. En `fetch.rs` se llama
`validate_resolved_addresses(&ips)` (`:174`) y luego `.resolve_to_addrs(host, &sorted_addresses)`
(`:242`) sin revalidación. No existe otro filtro. La sub-auditoría de backend marcó
`public_v6` como "unusually thorough/CLEAN", pero leyó `to_ipv4_mapped` por código; **no**
probó la forma compatible. La prueba compilada refuta esa conclusión.

**Impacto**: `POST /deep` acepta `targets[].url` **suministrado por el cliente**
(`DeepTarget.url`, `crates/amatl-core/src/service.rs:172-177`; `deep_selected` lo valida como
*string* con `validate_search_url`, que solo mira IPs literales y sufijos de hostname). Un
cliente con scope Deep que controle la resolución DNS de un hostname (o use un hostname que
resuelva a un AAAA IPv4-compatible) consigue que el fetcher conecte a loopback/RFC1918/metadata
cloud. La única barrera es `is_public_ip`, que la deja pasar. Explotabilidad real depende del
control de DNS, pero la defensa está rota: es un SSRF bypass, no una hipótesis.

**Corrección** (una línea, `security.rs`):
```rust
fn public_v6(address: Ipv6Addr) -> bool {
    if let Some(mapped) = address.to_ipv4_mapped() { return public_v4(mapped); }
    // IPv4-compatible ::x.x.x.x (deprecada) — mismas reglas que su IPv4 embebida.
    if let Some(compat) = address.to_ipv4() { return public_v4(compat); }
    ...
}
```
`to_ipv4()` cubre *tanto* mapped como compatible. Añadir test de regresión con
`::127.0.0.1`, `::10.0.0.1`, `::169.254.169.254` en `security.rs::tests`.
(Opcional, menor: bloquear también NAT64 `64:ff9b::/96` y 6to4 `2002::/16`, que hoy pasan.)

---

### CHAIN-001 — Advisories sin remediar: `cargo audit` y `cargo deny check` fallan

**Ubicación**: `Cargo.lock`; `deny.toml` (sin lista `ignore`).

**Evidencia** (`cargo audit`, exit 1):
```
libsqlite3-sys 0.30.1  RUSTSEC-2026-0459  6.8 medium  (fix >=0.39.0)
quinn-udp      0.5.15  RUSTSEC-2026-0538              (fix >=0.5.16 / >=0.6.3)
rmcp           3.1.2   RUSTSEC-2026-0462  6.5 medium  (fix >=3.9.0)
rmcp           3.1.2   RUSTSEC-2026-0461  7.5 high    (fix >=3.6.0)
rmcp           3.1.2   RUSTSEC-2026-0457              (fix >=3.7.0)
rustix         1.1.4   RUSTSEC-2026-0486  unsound (warning)
error: 5 vulnerabilities found!
```
`cargo deny check` → `advisories FAILED, bans ok, licenses ok, sources ok` (exit 1).
`deny.toml` **no** contiene entradas `ignore`, así que ambas puertas están rojas.

**Refutación intentada**: `docs/security/supply-chain.md:96-117` afirma "`cargo audit` is
green" y "`cargo deny check` is clean without any `ignore` entry". Eso era cierto en
2026-10-26 (commit `ce172d9d`, según CONTINUIDAD); las advisories de `rmcp`/`quinn-udp`/
`rustix`/`libsqlite3-sys` son del **2026-10-20 → 2026-10-27**, posteriores a esa medición, y
ya entraron al grafo. `rmcp 3.10.0` **es alcanzable** (`cargo update -p rmcp --dry-run` lo
propone), luego la remediación no está bloqueada. No hay waiver escrito (ADR) que cubra estas
5.

**Impacto**: `.github/workflows/ci.yml` ejecuta `cargo audit` (línea 46) y `cargo deny check`
(línea 47) en **cada push/PR** ⇒ el job `contract-gate` está rojo. Además `release.yml` las
repite. Remediation SLA de `SECURITY.md`: high→30d, medium→60d (desde 2026-10-20/22, ya
cercano/vencido para las de severidad alta/media).

**Corrección**: `cargo update -p rmcp` (a ≥3.9.0), `-p quinn-udp`, `-p rustix`,
`-p libsqlite3-sys` (verificar compat de `sqlx 0.8`); re-ejecutar audit+deny. Si alguna no
tiene fix compatible, **no** silenciar: registrar el caso en `supply-chain.md` y (solo con ADR
firmado por `@IA-Alex`) un `ignore` justificado — la política actual prohíbe el allowlist
silencioso.

---

### BUILD-001 — `cargo fmt --check` falla en código commiteado

**Ubicación**: `crates/amatl-cli/tests/cli.rs:188,196`; `crates/amatl-core/src/service.rs:3186`.

**Evidencia** (`cargo fmt --all -- --check`, exit 1, 4 diffs):
```
Diff in crates/amatl-cli/tests/cli.rs:188:
-    let base =
-        std::env::temp_dir().join(format!("amatl-canary-unshipped-{}-{id}", std::process::id()));
+    let base = std::env::temp_dir().join(format!(
+        "amatl-canary-unshipped-{}-{id}",
+        std::process::id()
+    ));
Diff in crates/amatl-core/src/service.rs:3186:
-            Err(ProviderCanaryError::UnknownProvider("unshipped_source".into()))
+            Err(ProviderCanaryError::UnknownProvider(
+                "unshipped_source".into()
+            ))
```

**Refutación intentada**: comprobé que el árbol está limpio (`git status --porcelain` vacío),
así que el fallo está **commiteado**, no es un cambio local. Los diffs son solo de formato
(exclusivamente en código de test), pero `cargo fmt --all -- --check` es la **primera** línea
del `contract-gate` (`.github/workflows/ci.yml:31`) y de `release.yml:31`.

**Impacto**: CI rojo por pura disciplina de formato; el proyecto lo trata como no negociable
(`mem:task_completion`). Corrección trivial: `cargo fmt --all`.

---

## 5. Hallazgos Medio/Bajo (resumen)

- **DOC-001** — `docs/security/supply-chain.md:96` ("cargo audit is green") y `:116-117`
  ("clean without any ignore entry") son **falsos hoy** (ver CHAIN-001). Corregir el texto o
  remediar; no dejar la afirmación sin fecha de verificación.
- **TEST-001** — En la **primera** ejecución de `cargo test --workspace` observé
  `passed=417 failed=1` (concurrencia con un `cargo doc` simultáneo); las **5** ejecuciones
  siguientes dieron `695 passed, 0 failed`. Es un flake no caracterizado (la memoria describe
  flakes previos ya remediados en `ETXTBSY` y deadlines de 1000ms). Causa **NO VERIFICADA**:
  no pude aislar el test. Recomendado: correr la suite en bucle con
  `--no-fail-fast` y capturar el nombre, o revisar tests con `sleep`/deadline ajustado
  (`execution.rs:1205`, `server/tests.rs:914,1076`).
- **CHAIN-002** — `crates/amatl-core/tests/supply_chain_contract.rs` mantiene floors solo
  para `quinn-proto`, `rustls`, `rustls-webpki`, `tokio`, `webpki-roots`. **Ninguna** de las 5
  advisories abiertas hoy está nombrada, luego el guardián offline (que corre en macOS/Windows,
  sin `cargo audit`) no las detecta. Añadir floors a medida que se remedien (el propio
  doc-comment del test lo exige).
- **FE-001** — `app.js:821` `decodeURIComponent(value)` sin `try/catch` dentro de
  `restoreFromLocation()`, llamada sin guardia en `DOMContentLoaded` (`app.js:2007`). Un URL
  como `/?q=%` lanza `URIError` y aborta la restauración desde enlace. Sin impacto de
  seguridad; robustez. Envolver en `try/catch`.
- **FE-002** — `app.js:1055` interpola `provider.status` sin validar en `className`
  (asignación de propiedad, no HTML ⇒ no XSS; sí tokens de clase arbitrarios). Normalizar a
  una allowlist.
- **BE-001** — `robots.rs:285-287` vacía la caché entera al alcanzar `max_entries` (64) en vez
  de evict-uno: 64 orígenes distintos fuerzan re-fetch de todos los `robots.txt`. Nit de
  rendimiento acotado al crawl.
- **DOC-002** — Citas muertas: `docs/security/hallazgos-2026-10-26.md` referencia
  `docs/api/openapi.yaml` (el real es `crates/amatl-server/openapi.yaml`); y
  `amatl_revision_critica.md` H5 cita `chacha20 0.10.1` yanked, cuando el lockfile ya tiene
  `0.10.2` (H5 obsoleto).

---

## 6. Lo que se descartó (comprobado sin hallazgo)

- **Autenticación** (`amatl-server/src/lib.rs`): token solo por env, mínimo 32 bytes,
  comparación en tiempo constante sobre digest SHA-256, sin early-exit; ninguna ruta protegida
  omite auth; `required_scope` es fail-closed (default `Admin`); identidad nunca desde
  cabeceras (X-Forwarded-For se ignora a propósito). **CLEAN**.
- **SSRF (resto)**: `resolve_to_addrs` fija las IPs validadas (cierra TOCTOU de DNS rebinding);
  redirects deshabilitados en el cliente y re-validados por salto con re-resolución; allowlist
  de cabeceras de 5 entradas, sin CRLF. **CLEAN** salvo SEC-001.
- **Panics remotos**: revisados todos los `unwrap/expect/index` alcanzables; ninguno
  disparable desde una petición. Producción tiene solo ~12 `unwrap/expect` fuera de tests,
  todos justificados. **CLEAN**.
- **Frontend XSS**: cero `innerHTML`/`insertAdjacentHTML`/`eval`/`document.write`; todo por
  `textContent`; `safeHttpUrl` bloquea `javascript:`/`data:`; `rel="noopener noreferrer"` en
  enlaces; token solo en memoria (input DOM), limpiado en `pagehide`. **CLEAN**.
- **CSP y cabeceras**: `script-src 'self'` sin `unsafe-*`, `object-src 'none'`,
  `frame-ancestors 'none'`; sin script/style inline; `nosniff`, `X-Frame-Options: DENY`,
  `Referrer-Policy`, HSTS bajo HTTPS. **CLEAN**.
- **i18n**: paridad `es`/`en` = 222 claves cada uno, diff simétrico vacío. **CLEAN**.
- **Benchmark gate**: `benchmark ranking-v2 --json` → `"passed": true` (ndcg@3 0.6558→0.9194).
- **clippy/rustdoc**: ambos limpios con `-D warnings`.
- **Secretos/versionado**: `amatl.toml`, `*.sqlite3`, `.env` no están en git (verificado).
- **SQL**: 67 usos de `sqlx::query`; los `format!` en storage son rutas de fichero y mensajes
  de error, no concatenación SQL con entrada de usuario.

---

## 7. Límites de esta auditoría

- **Volumen**: revisados los módulos de seguridad/auth/server/UI en profundidad; del resto de
  `amatl-core` (35k LOC) se leyó el mapa de símbolos y se apuntó a los puntos calientes, no
  cada función. Estimación de cobertura nominal del núcleo: ~40% leído, 100% barrido por
  grep/estructura.
- **Truncado**: el informe completo va en este fichero (no truncado en chat). Las salidas
  largas de `cargo` (p.ej. `deny check`) se resumieron; el crudo está en el historial de la
  sesión.
- **Verificación**: SEC-001 confirmado con programa compilado, no solo lectura. TEST-001 es un
  **síntoma** observado, causa NO VERIFICADA.
- **Dinámico**: sin navegador real (no se ejecutó `browser_e2e`), sin WebDriver, sin red real,
  sin CI remoto, sin macOS/Windows. El estado de advisories cambia con RustSec; válido a
  2026-11-13.
- **Permisos**: no se modificó ningún fichero del proyecto (solo se creó este informe en
  `audit/`).

---

## 8. Roadmap priorizado

**Inmediato (S)**
1. `BUILD-001`: `cargo fmt --all` → desbloquea la primera línea del gate.
2. `SEC-001`: `to_ipv4()` en `public_v6` + tests de regresión.
3. `CHAIN-001`: `cargo update` de `rmcp`/`quinn-udp`/`rustix`/`libsqlite3-sys`; re-verificar
   audit+deny. Documentar en `supply-chain.md` (corrige `DOC-001`).

**Corto plazo (S-M)**
4. `CHAIN-002`: añadir floors offline para las advisories remediadas.
5. `DOC-001`/`DOC-002`: sincronizar la documentación de seguridad con el estado medido y
   corregir citas muertas.
6. `TEST-001`: caracterizar/eliminar el flake.

**Estructural (M)**
7. `FE-001`/`FE-002`/`BE-001`: robustez de UI y caché de robots.
8. Considerar NAT64/6to4 en el filtro SSRF, y añadir un job de audit programado con más
   frecuencia que el nightly actual, dado que las advisories entraron en una semana.

---

## 9. Preguntas abiertas

- ¿Las 5 advisories tienen fix compatible dentro de las restricciones de `sqlx 0.8` /
  `axum-server 0.8`? (`cargo update` dry-run sugiere que sí para `rmcp`; falta probar el resto).
- ¿El fallo intermitente de TEST-001 fue interferencia por correr `cargo doc` en paralelo sobre
  el mismo `target/`, o un flake real de la suite? Requiere reproducir en aislamiento.

---

## 10. Estado tras la reparación (2026-11-13)

Cerrados y verificados:

- **SEC-001** — `public_v6` usa `to_ipv4()` (cubre mapped y compatible) y rechaza NAT64/6to4.
  Tests de regresión añadidos. `cargo test` verde.
- **BUILD-001** — `cargo fmt --all`; el gate de formato queda en verde.
- **CHAIN-001 (4 de 5)** — `rmcp` 3.10.0, `rustix` 1.1.6, `quinn-udp` 0.5.16.
- **CHAIN-002** — floors añadidos para `rmcp`, `rustix`, `quinn-udp`.
- **DOC-001 / DOC-002** — `supply-chain.md` reescrito con la re-auditoría; H5 corregido.
  (`docs/api/openapi.yaml` se confirmó como cita histórica correcta de C1, no un defecto.)
- **FE-001 / FE-002 / BE-001** — corregidos con tests.
- **MCP** — adaptado a rmcp 3.10 (`known_up_to(LATEST)`); handshake arreglado; 3 tests
  actualizados al contrato de SEP-2567.

Abierto por decisión del operador (sin waiver):

- **CHAIN-001 (`libsqlite3-sys` RUSTSEC-2026-0459)** — sin fix compatible alcanzable;
  registrado en `supply-chain.md`; `cargo audit` / `cargo deny check` permanecen rojos
  **solo** por esta advisory.

**TEST-001** no se reprodujo en 5 ejecuciones aisladas; consistente con interferencia de
`cargo doc` concurrente sobre el mismo `target/`, no con un flake del código. Se deja como
observación, no como defecto confirmado.

