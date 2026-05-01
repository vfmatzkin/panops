# Security Policy

## Supported versions

panops is pre-alpha. Only the `main` branch receives fixes. There is no released version yet.

## Reporting a vulnerability

Email panops@tzk.ar with `[panops security]` in the subject, or use GitHub's [private vulnerability reporting](https://github.com/vfmatzkin/panops/security/advisories/new).

Include:

- Affected component (crate, sidecar, IPC surface).
- Repro steps or proof of concept.
- Impact you observed.

I'll acknowledge within a week and keep you posted on the fix. Please don't open a public issue for anything that looks exploitable.

## Scope

panops is local-first and never phones home. The threat model is mostly local: malicious input fixtures, IPC misuse from another process on the same machine, and tampering with on-disk meeting databases. Network-facing concerns are out of scope (there's no network surface).

If you find a third-party dependency with a known CVE that affects panops, file it as a regular `type:debt` issue with `severity:high` or above instead of using this channel.
