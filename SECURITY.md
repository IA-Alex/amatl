# Security Policy

## Supported versions

AMATL has not published a SemVer release. Security fixes are therefore made
only on the current `main` branch. This table must be updated when the first
release is published.

| Version | Supported |
|---|---|
| Unreleased (`main`) | Yes |
| Any untagged historical revision | No |

## Reporting a vulnerability

Do not open a public issue, discussion, or pull request for a suspected
vulnerability. **Pending owner definition:** the repository owner must publish a
private security-reporting channel before enabling public deployment or public
contributions. No PGP key or email address is asserted by this repository.

Until that channel exists, coordinated private reporting is operationally
blocked. Do not send secrets or exploit details to an unverified address.

Include the affected revision, surface (`CLI`, HTTP API, MCP, UI, provider, or
Deep), reproduction steps, impact, relevant logs with secrets removed, and any
suggested mitigation.

## Response and remediation

Acknowledgement and remediation service levels are **pending owner definition**.
The owner must publish severity-based targets, a responsible role, and an
escalation path; the project does not claim an SLA it cannot currently meet.
Once a private report is accepted, maintainers should minimize disclosure,
confirm scope, agree on a disclosure date with the reporter, prepare tests and
a fix, and publish a security advisory when users can act on it.

## Scope

In scope:

- the Rust workspace and versioned configuration;
- CLI, embedded UI, HTTP API, and MCP surface;
- Query parsing, provider adapters, routing, Budget, persistence, and Deep;
- SSRF, DNS rebinding, redirects, resource exhaustion, secret exposure,
  authentication, Host/Origin/CORS validation, and dependency risks.

Out of scope as implementation claims, but useful as design reports:

- third-party provider services, their availability, data, and infrastructure;
- social engineering or denial of service against third-party providers;
- Chromium rendering, because the renderer remains unavailable until verifiable
  CDP isolation and resource controls exist (`render.rs`);
- infrastructure not maintained in this repository.

Test only systems and accounts you own or have explicit permission to test. Do
not access private data, degrade service, persist access, or exfiltrate secrets.

## Safe harbor and coordinated disclosure

The project intends not to pursue legal action against good-faith research that
follows this policy, avoids privacy harm and service disruption, reports
privately, and allows a reasonable remediation period. This statement does not
authorize testing of third parties or waive their terms. Public disclosure
should occur on the mutually agreed date or after the project has clearly
failed to engage through its published channel.

## Recognition

A public hall of fame may be created only with each reporter's explicit consent.
It does not currently exist.

The detailed controls and residual risks are indexed from
[`docs/seguridad.md`](docs/seguridad.md).
