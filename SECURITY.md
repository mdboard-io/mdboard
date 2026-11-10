# Security policy

## Supported versions

Security fixes target the latest 0.1.x release. Pre-release builds older than the current release
are not supported.

## Reporting

Do not open a public issue for a vulnerability. Use GitHub’s private security advisory flow:

<https://github.com/mdboard-io/mdboard/security/advisories/new>

Include:

- the mdboard version;
- a minimal reproduction;
- the affected surface (CLI, HTML server, Git actions, plugin verification);
- impact and any known mitigation.

Never include live tokens, private board content, customer data or credentials.

## Scope

In scope:

- authentication and authorization bypasses in `serve`;
- CSRF or origin-check bypasses;
- path traversal or unintended file writes;
- command injection in CLI/Git integration;
- plugin digest/lock verification bypasses;
- privilege escalation in actor/role handling.

Out of scope for 0.1.0:

- internet-facing hardening (the server is local-only);
- OIDC/trusted-proxy behavior (not implemented);
- plugin execution (disabled);
- denial of service by a local user with repository write access.

## Security model

The full model is in the website’s [Security reference](https://mdboard.io/reference/security) and
the repository’s ADRs. In short: local-only server, one-time token, HttpOnly session cookie, CSRF,
review independence, owner-only files and a closed plugin default.

## Disclosure

We will acknowledge a report, assess impact, prepare a fix and coordinate disclosure. Please give
us reasonable time before publishing details.
