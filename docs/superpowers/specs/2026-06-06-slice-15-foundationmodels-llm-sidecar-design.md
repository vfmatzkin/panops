# Slice 15 — FoundationModels LLM Sidecar (Anchor A completion)

**Status:** design approved 2026-06-06 (maintainer). Brainstorm: this file. Plan: forthcoming via `superpowers:writing-plans`.

## Problem

Notes generation runs through the `LlmProvider` port. The only real adapter today is `GenaiLlm` (`crates/panops-portable/src/genai_llm.rs`, via the `genai` crate → a local Ollama `gemma3:4b`), built inline in `crates/panops-engine/src/server/mod.rs::n()`. For a signed, notarized, clean-Mac `.app` (v0.1 criterion #6) the product should generate notes **on-device** with Apple's FoundationModels (no separate Ollama install), while still working where FoundationModels isn't available.

## Goal

An on-device LLM via a Swift `panops-llm-mac` sidecar, selected automatically when available, falling back to the existing Ollama path otherwise. Completes **Anchor A** (Mac shell + sidecars). No change to the notes pipeline or the `LlmProvider` contract.

## Decisions (locked)

- **Option B — FoundationModels + Ollama fallback** (maintainer, on macOS 26). Auto-detected; no user-facing env var.
- **Guided generation.** Convert `LlmRequest.schema` → `DynamicGenerationSchema` at runtime, `respond(to:schema:)`, serialize the structured `GeneratedContent` → JSON. Constrains output to the schema (valid-by-construction). Not text-JSON parsing.
- **Resolver-time availability probe.** `pick_llm` spawns the sidecar once at startup and probes `SystemLanguageModel.availability`; available → FoundationModels, else → Ollama. One-time cost; clean startup decision.
- **Mirror the ASR sidecar** (`whisperkit_asr.rs` / `asr_resolver.rs` / `apps/panops-asr-mac/`) for spawn/stdio/JSON-RPC/Drop/respawn + resolver.

## Scope

### In
- `crates/panops-mac/src/foundation_llm.rs` — `FoundationLlm` impl `LlmProvider`; lazy-spawn `panops-llm-mac`, JSON-RPC over stdio (`complete`, `probe`), reuse, `Drop` closes stdin, respawn on broken pipe. Reuses the spawn/stdio machinery shape from `whisperkit_asr.rs`.
- `crates/panops-engine/src/llm_resolver.rs` — `pick_llm()` mirroring `asr_resolver::pick_asr`; extracts the inline build from `server/mod.rs::n()`.
- `apps/panops-llm-mac/` SwiftPM executable — `Sources/PanopsLlmMac/{main.swift, Codecs.swift, Generator.swift}`: stdio JSON-RPC loop; `Generator` wraps `LanguageModelSession`; `Codecs` does JSON-Schema ↔ `DynamicGenerationSchema` and `GeneratedContent` → JSON.
- Engine wiring: `server/mod.rs::n()` → `llm_resolver::pick_llm(...)`; `main.rs` passes the sidecar path (env gate dev/CI).
- Tests: `FoundationLlm` against the `LlmProvider` conformance suite via a fake sidecar binary; Swift unit tests for the codec.

### Out (file as debt if surfaced)
- Streaming / partial tokens (notes generation is batch).
- Per-request model/provider selection (#98 — separate).
- Production `Bundle.main` sidecar resolution + notarization (lands in the packaging slice; this slice keeps the env gate).
- Multi-turn session reuse beyond one `complete` (each `complete` is stateless, matching the port).

## Architecture

```
panops-core (LlmProvider port; UNTOUCHED)
        ▲
        │ impl
crates/panops-engine/src/llm_resolver.rs ── pick_llm() ──┐
        │  macOS + sidecar available + probe OK           │ else
        ▼                                                 ▼
crates/panops-mac/FoundationLlm  ──spawn/stdio JSON-RPC──►  panops-portable/GenaiLlm (Ollama)
        │
        ▼
apps/panops-llm-mac (Swift): main ↔ Generator(LanguageModelSession) ↔ Codecs(DynamicGenerationSchema/GeneratedContent)
```

## Wire protocol (JSON-RPC 2.0 over stdio, mirrors ASR sidecar)

- `probe` → `{ available: bool, reason?: string }` (maps `SystemLanguageModel.availability`).
- `complete` params `{ system?, user, schema?, temperature, max_tokens }` → result `{ json: <object> }` (guided) or `{ text: <string> }` (no schema) → `LlmResponse::Json | Text`. Errors map to `LlmError` (`Provider`/`EmptyResponse`/`InvalidSchema`).

## Data flow (one `complete`)

1. Pipeline calls `LlmProvider::complete(req)` → `FoundationLlm`.
2. Adapter ensures the sidecar is spawned, writes one JSON-RPC line, reads one response line.
3. Swift: `Codecs` builds `DynamicGenerationSchema` from `req.schema` → `GenerationSchema(root:dependencies:)`; `Generator` runs `session.respond(to: req.user, schema:)` (system → session instructions); `Codecs` walks `GeneratedContent` → JSON.
4. Adapter returns `LlmResponse::Json`.

## Testing

- **Unit (Rust, cheap):** `FoundationLlm` ↔ fake sidecar binary (a stub that speaks the JSON-RPC protocol with canned `probe`/`complete` responses) — passes the same `LlmProvider` conformance suite as `GenaiLlm`/`MockLlm`. Resolver-probe fallback path tested with a fake that reports `available:false`.
- **Unit (Swift):** `Codecs` round-trips for a few schema shapes (object, enum→anyOf, nested, optional) and `GeneratedContent`→JSON.
- **Heavy/gated:** a real-FoundationModels smoke (macOS 26 only, gated like the WhisperKit conformance) producing a valid notes-section JSON. The #149 prompt-regression gate already guards prompt drift.

## Three-tier boundaries

### ✅ Always do
- Run `cargo fmt --all && cargo build --workspace --locked && cargo test --workspace --locked && cargo clippy --workspace --all-targets --locked -- -D warnings` per task; Swift sidecar built with `swift build --configuration release` + `swift test`.
- Keep `panops-core` platform-free; the adapter is `#[cfg(target_os="macos")]` in `panops-mac`.
- Open issues for any deferred item; commit per plan task.
- Verify pushed == local before relying on CI.

### ⚠️ Ask first
- Changing the `LlmProvider` trait signature or `LlmRequest`/`LlmResponse` shape.
- Dropping or replacing the Ollama/`GenaiLlm` fallback.
- Adding any new runtime dependency to `panops-core` or `panops-portable`.
- Changing the `genai`/Ollama default model.

### 🚫 Never do
- Introduce a trait without one real impl + one fake.
- Any network egress / telemetry from the sidecar (on-device only).
- A user-facing env var for config (the `PANOPS_LLM_SIDECAR_BIN` gate is dev/CI-only, flagged here).
- Open or merge the PR autonomously.

## Acceptance criteria

1. On macOS 26 with the sidecar present + FoundationModels available, `notes.generate` produces valid structured notes via the sidecar (no Ollama).
2. With the sidecar absent OR `probe` reporting unavailable, the engine logs the reason and falls back to `GenaiLlm` (Ollama) — notes still generate.
3. `panops-core` unchanged; `cargo test --workspace` green; `FoundationLlm` passes the `LlmProvider` conformance suite.
4. No network egress from the sidecar; no new user env vars.

## Risks

- **`GeneratedContent` → JSON** mapping for nested/array shapes — covered by Codec unit tests; the notes schemas are shallow (title/narrative/key_points/action_items).
- **macOS 26 availability** — model may be downloading/unavailable; the resolver probe handles it via fallback.
- **`server/mod.rs` conflict with #141** — both touch `n()`/wiring → sequence this slice **after #141 merges**.

## Open questions (deferred)

- Production `Bundle.main` resolution + notarized entitlements → packaging slice.
- Whether to expose temperature/max_tokens per provider → tie to #98.
