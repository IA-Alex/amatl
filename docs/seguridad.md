# Seguridad de AMATL

Este documento resume para el equipo la política normativa en
[`SECURITY.md`](../SECURITY.md). Ante una diferencia, prevalece la versión en
inglés.

## Reporte privado

No abras un issue, discusión o pull request público con detalles de una
vulnerabilidad. Usa el canal privado y los tiempos de respuesta publicados en
[`SECURITY.md`](../SECURITY.md): el correo verificado del propietario. Los
Security Advisories de GitHub se añadirán cuando el plan del repositorio los
habilite. No hay clave PGP publicada; no envíes detalles a otros canales.

El reporte debe incluir revisión afectada, superficie, pasos de reproducción,
impacto, logs ya depurados de secretos y mitigación sugerida. `@IA-Alex` es el
responsable; el SLA por severidad cubre acuse, triage, corrección y escalación.

## Alcance

Incluye workspace Rust, configuración, CLI, UI, API, MCP, adapters, Budget,
persistencia, Deep, SSRF, DNS rebinding, redirects, agotamiento, secretos,
autenticación y cadena de suministro. Excluye la infraestructura y servicios de
terceros. Chromium no es una capacidad activa: permanece bloqueado hasta contar
con aislamiento CDP verificable.

La investigación de buena fe debe limitarse a sistemas autorizados, evitar daño
y mantener divulgación coordinada. La intención del proyecto es no iniciar
acciones legales contra quien cumpla esas condiciones; esto no concede permiso
sobre sistemas de terceros.

## Documentos técnicos

- [Modelo de amenazas](security/threat-model.md)
- [Matriz OWASP ASVS](security/asvs-checklist.md)
- [Controles SSRF](security/ssrf-controls.md)
- [Hardening HTTP](security/http-hardening.md)
- [Gestión de secretos](security/secrets.md)
- [Cadena de suministro](security/supply-chain.md)
- [Retención de datos](security/data-retention.md)
