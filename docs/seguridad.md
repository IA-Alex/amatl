# Seguridad de AMATL

Este documento resume para el equipo la política normativa en
[`SECURITY.md`](../SECURITY.md). Ante una diferencia, prevalece la versión en
inglés.

## Reporte privado

No abras un issue, discusión o pull request público con detalles de una
vulnerabilidad. **Pendiente de definición por el propietario:** publicar un
canal privado verificable antes de desplegar o aceptar contribuciones públicas.
No existe en el repositorio un correo, clave PGP ni SLA autorizado que pueda
documentarse sin inventarlo.

El reporte debe incluir revisión afectada, superficie, pasos de reproducción,
impacto, logs ya depurados de secretos y mitigación sugerida. Los objetivos de
acuse y corrección, el rol responsable y la escalación también quedan pendientes
de definición por el propietario.

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
