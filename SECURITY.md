# Security Policy

## Supported versions

Security fixes are made on the current `main` branch and the active release
candidate. Release assets are authoritative only when published from the
matching annotated tag.

| Version | Supported |
|---|---|
| `0.1.0-rc.1` | Yes |
| Unreleased (`main`) | Yes |
| Any untagged historical revision | No |

## Reporting a vulnerability

Do not open a public issue, discussion, or pull request for a suspected
vulnerability. Email
[`alexishernande87@hotmail.com`](mailto:alexishernande87@hotmail.com?subject=AMATL%20security%20report)
with subject `AMATL security report`. This address is the verified owner contact
used by the repository history; no PGP key is currently published. The
repository owner and security maintainer is
[`@IA-Alex`](https://github.com/IA-Alex), as recorded in `.github/CODEOWNERS`.

GitHub's Security Advisory and Private Vulnerability Reporting endpoints are
not available for the repository's current private-plan configuration. When
GitHub enables them, they become an additional channel without replacing this
verified email. Do not send secrets or exploit details through ordinary issues
or to any other address.

Include the affected revision, surface (`CLI`, HTTP API, MCP, UI, provider, or
Deep), reproduction steps, impact, relevant logs with secrets removed, and any
suggested mitigation.

## Response and remediation SLA

`@IA-Alex` owns acknowledgement, triage, remediation and coordinated disclosure.
Targets start when a complete report reaches the private reporting channel:

| Severity | Acknowledge | Triage | Mitigation or remediation target |
|---|---:|---:|---:|
| Critical | 1 business day | 2 business days | mitigation in 7 calendar days; remediation in 14 |
| High | 2 business days | 5 business days | remediation in 30 calendar days |
| Medium | 5 business days | 10 business days | remediation in 60 calendar days |
| Low | 10 business days | 20 business days | next planned release or 90 calendar days |

Severity follows impact and exploitability, using CVSS as supporting evidence
rather than an automatic decision. If a target cannot be met, the owner records
the reason, compensating control and revised date in the same private thread.
The reporter may escalate an overdue report by adding a new comment that
mentions `@IA-Alex` or replying with `AMATL security escalation`; disclosure
dates remain coordinated privately.

Maintainers minimize disclosure, confirm scope, agree on a disclosure date with
the reporter, prepare regression tests and a fix, and publish a GitHub Security
Advisory when users can act on it.

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
- Chromium rendering when disabled or unavailable; reports against an enabled
  renderer and its isolation boundary are in scope;
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
