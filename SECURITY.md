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

panops is local-first and never phones home. The threat model is mostly local: malicious input fixtures, IPC misuse from another process on the same machine, and tampering with on-disk meeting databases.

If you find a third-party dependency with a known CVE that affects panops, file it as a regular `type:debt` issue with `severity:high` or above instead of using this channel.

## Network surface

panops itself runs locally and does not phone home. Two optional surfaces involve external endpoints:

- **Model downloads** (`ASR` and diarization). On first run, the engine downloads pinned-hash models from the URLs in `crates/panops-portable/src/model.rs`. SHA-256 verified.
- **LLM provider for notes generation** (slice 04+). When `panops notes` runs, the configured LLM provider receives the transcript text + screenshot timestamps. Provider auto-detects from env vars: `ANTHROPIC_API_KEY` → Anthropic, `OPENAI_API_KEY` → OpenAI, `OLLAMA_HOST` → Ollama. **Audio bytes and screenshot images are never sent to any LLM provider** — only text and timestamps. If you set none of those env vars, the engine errors and no network call is made.

When using cloud providers, the transcript text and section/title/tag prompts go to that provider per their privacy terms. For zero external surface, use a local Ollama instance.
