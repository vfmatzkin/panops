# Slice 04 — Notes generation: design

**Status**: Locked 2026-05-01.
**Goal**: End-to-end pipeline from a diarized transcript + a screenshot stream to a markdown notes file with embedded screenshots, via a multi-call LLM pipeline that produces a typed intermediate representation rendered deterministically per `MarkdownDialect`.

This slice introduces the project's first LLM port and the first port whose output is user-visible content rather than structured data. It is the foundation that later prompt-iteration, ensemble, verifier, and cross-meeting-context work builds on. Slice 04 ships less LLM cleverness and more architecture: the IR, the per-stage prompt boundaries, the dialect-aware exporter, and a working baseline pipeline that produces clean (if shallow) notes from the slice 03 multi-speaker fixture.

## Why this shape

The main design spec sketches `LlmProvider::summarize(transcript, screenshots, template) → markdown` as a single call. We diverge: a single mega-prompt is brittle (no per-stage validation, no per-role model swap, output drift on minor prompt edits). The structured-IR pipeline:

- Locks in the IR shape early so prompt iteration in later slices doesn't churn ports.
- Lets each stage swap models independently (small/fast for extraction, smarter for narrative).
- Makes prompts unit-testable via `MockLlm` that pattern-matches prompt fingerprints to canned responses.
- Renders markdown deterministically in Rust, eliminating a class of formatting bugs from LLM output.

Cross-meeting context, speaker-name resolution, verifier passes, Slack summary, transcription corrections, and ensemble voting are explicitly out of scope here — slice 04.5+ will plug them into this architecture without touching ports.

## Ports introduced

### `LlmProvider` (low-level)

```rust
pub trait LlmProvider: Send + Sync {
    fn complete(&self, req: LlmRequest) -> Result<LlmResponse, LlmError>;
}

pub struct LlmRequest {
    pub system: Option<String>,
    pub user: String,
    pub schema: Option<JsonSchema>,   // when set, response must validate against it
    pub temperature: f32,             // 0.0..=2.0; default 0.2 for extraction, 0.6 for narrative
    pub max_tokens: u32,
}

pub enum LlmResponse {
    Text(String),
    Json(serde_json::Value),
}

pub enum LlmError {
    Network(String),
    InvalidSchema { expected: String, got: String },
    EmptyResponse,
    Provider(String),       // adapter-specific, opaque
    Cancelled,
}
```

Adapters: `MockLlm` (in `panops-core::conformance::fakes`, deterministic, prompt-fingerprint keyed) and `GenaiLlm` (in `panops-portable`, wraps `rust-genai` 0.4 with provider auto-detection: env vars for cloud keys, `OLLAMA_HOST` for local, future Apple-FM through the sidecar).

`MockLlm` is the conformance fake: tests register `(prompt_fingerprint, response)` pairs; unmatched prompts panic loudly so tests catch prompt drift. Fingerprint = sha256 of `system + user`.

### `NotesGenerator` (high-level pipeline)

`NotesGenerator` is **not a trait** in slice 04 — it's a single concrete pipeline in `panops-core::notes` parameterised by an `&dyn LlmProvider`. It will become a trait if we ever need a non-LLM path (e.g. fully template-rendered notes); YAGNI for now.

```rust
pub struct NotesGenerator<'a> {
    pub llm: &'a dyn LlmProvider,
    pub dialect: MarkdownDialect,
}

impl NotesGenerator<'_> {
    pub fn generate(&self, input: NotesInput) -> Result<StructuredNotes, NotesError>;
}

pub struct NotesInput {
    pub transcript: Vec<Segment>,           // diarized segments from slice 03
    pub screenshots: Vec<Screenshot>,       // (ms_since_start, file_path, optional_caption)
    pub meeting_metadata: MeetingMetadata,  // started_at, duration_ms, source_path, language hint
}
```

### `NotesExporter` (output)

```rust
pub trait NotesExporter: Send + Sync {
    fn export(&self, notes: &StructuredNotes, dest: &Path) -> Result<ExportArtifact, ExportError>;
}

pub struct ExportArtifact {
    pub primary_file: PathBuf,             // .../notes.md
    pub assets: Vec<PathBuf>,              // copied / referenced screenshots
}
```

Slice 04 ships one impl: `MarkdownExporter` in `panops-portable`, writing `<dest>/notes.md` + a sibling `screenshots/` dir of referenced images. Future `NotionExporter` is a separate adapter.

## Domain types

### `MarkdownDialect`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MarkdownDialect {
    NotionEnhanced,    // default
    Basic,             // CommonMark only
}
```

- `NotionEnhanced`: emits `<callout icon="🎯">…</callout>`, `<details><summary>…</summary>…</details>`, `<table>` blocks, `{color="..."}` block colors. Renders cleanly in Notion (POSTable to `/v1/pages` for the future Notion exporter), degrades gracefully in renderers that ignore unknown HTML-like tags.
- `Basic`: CommonMark only (headings, lists, tables, code blocks, image embeds, plain blockquotes). For Obsidian / GitHub / vanilla viewers.

The dialect is passed as a structured arg to every LLM stage that emits markdown, alongside a per-dialect cheat-sheet string.

### `StructuredNotes` IR

```rust
pub struct StructuredNotes {
    pub schema_version: u32,                // 1 in slice 04
    pub frontmatter: NotesFrontmatter,
    pub sections: Vec<NotesSection>,
    pub language: LanguageCode,             // dominant language; LLM prompt language
    pub generated_at: DateTime<Utc>,
}

pub struct NotesFrontmatter {
    pub title: String,
    pub date: NaiveDate,
    pub started_at: DateTime<FixedOffset>,
    pub duration_ms: u64,
    pub speakers: Vec<String>,              // raw speaker_id labels; name-resolution is later slice
    pub tags: Vec<String>,                  // LLM-generated, lowercase, hyphenated
    pub template: String,                   // "default" in slice 04
    pub dialect: MarkdownDialect,
    pub panops_version: String,             // env!("CARGO_PKG_VERSION") of panops-engine
    pub source_audio: Option<PathBuf>,      // relative to notes dir
}

pub struct NotesSection {
    pub index: u32,                         // 1-based
    pub title: String,                      // narrative, not "Section 1"
    pub time_range_ms: (u64, u64),
    pub narrative_md: String,               // dialect-compliant markdown body
    pub key_points: Vec<String>,            // short bullets, may be empty
    pub action_items: Vec<ActionItem>,      // owner_TBD if unknown
    pub screenshots: Vec<Screenshot>,       // anchored to this section by timestamp
}

pub struct ActionItem {
    pub description: String,
    pub owner: Option<String>,              // None ⇒ "owner TBD"
    pub due: Option<NaiveDate>,
}

pub struct Screenshot {
    pub ms_since_start: u64,
    pub path: PathBuf,                      // absolute on input, relative on render
    pub caption: Option<String>,            // None until later slice
}
```

The IR is owned by `panops-core`. It's `Serialize + Deserialize` so engineers can dump intermediate state for debugging.

## Pipeline stages

`NotesGenerator::generate` runs five stages in order. Stage 2 is parallelised across sections; the rest are sequential.

### 1. Topic segmentation (deterministic)

Rule-based, no LLM call. Walks `Vec<Segment>`, opens a new section whenever:
- A silence gap > `TOPIC_GAP_MS` (default 8000) appears between consecutive segments, **or**
- The dominant speaker switches AND the new speaker has spoken < 10% of the previous section.

Minimum section length: `MIN_SECTION_MS` (default 30000). Below threshold, merge into the adjacent section.

For the slice 03 multi-speaker fixture (60s, A-B-A pattern, ~3 turns), this typically yields 1–3 sections. That's fine — testing exercises the pipeline shape, not segmentation quality.

LLM-based topic clustering is a slice 04.5+ concern.

### 2. Per-section narrative (LLM, parallel)

For each section, one LLM call. Prompt receives:
- The section's diarized transcript (speaker_id + text + timestamps).
- The active dialect's cheat-sheet (~150-token reference of allowed syntax).
- The strict speaker-attribution rule: never attribute a quote to a speaker unless segment metadata includes a confirmed `speaker_id` from diarization; otherwise use passive voice.
- Output language hint (the dominant language of the input, except: the user can override in `NotesInput::meeting_metadata.output_language` — slice 04 defaults to dominant; English-only mode is slice 04.5+).

Schema-constrained JSON response:

```json
{
  "title": "string",
  "narrative_md": "string (dialect-compliant)",
  "key_points": ["string", ...],
  "action_items": [{"description": "string", "owner": "string|null"}]
}
```

`temperature: 0.6`. Parallel via `rayon::par_iter` (CPU-bound on JSON parsing; the actual blocking is the HTTP I/O, but `rust-genai` is sync-blocking — fine for a CLI). Cap concurrency at 4 to avoid rate-limiting cloud providers.

If a section's LLM call fails, the section narrative falls back to a rendered transcript block (speaker_id: text, one line each) and `narrative_md` carries a `<!-- panops: llm error: ... -->` HTML comment in `NotionEnhanced`, or a similar plain-text marker in `Basic`. The pipeline does not abort — partial notes are better than no notes.

### 3. Screenshot anchoring (deterministic)

For each `Screenshot { ms_since_start }`, place it in the section whose `time_range_ms` contains that timestamp. If multiple, pick the one with `time_range_ms.0 ≤ ms < time_range_ms.1` (half-open). If none (rare, edge case at boundaries), attach to the nearest section by midpoint distance.

Blank/low-info filtering deferred to slice 04.5+.

### 4. Frontmatter + title + tags (LLM, single call)

One LLM call given:
- Section titles + key_points (compact summary; full narrative omitted to save tokens).
- Meeting metadata (date, duration, language).

Schema-constrained JSON response:

```json
{
  "title": "string",
  "tags": ["string", ...]
}
```

`temperature: 0.3`. Tags constrained: lowercase, kebab-case, max 10. Title <= 80 chars.

### 5. Render (deterministic)

`MarkdownExporter::export` walks the IR and emits markdown:

- YAML frontmatter (always, both dialects).
- Per-section: `## N. <title>` heading, `*[M:SS – M:SS]*` time-range italic line, `narrative_md`, then screenshots appended at section end as a 2-column gallery in `NotionEnhanced`, single-column image stack in `Basic`. Inline-by-paragraph placement is deferred to slice 04.5+.
- `---` separator between sections.

Screenshot files are copied (not symlinked) into `<dest>/screenshots/` and referenced with relative paths.

## File layout

```
crates/panops-core/src/
├── llm.rs              # LlmProvider trait, LlmRequest, LlmResponse, LlmError
├── notes/
│   ├── mod.rs          # public re-exports
│   ├── ir.rs           # StructuredNotes, NotesSection, ActionItem, Screenshot, NotesFrontmatter
│   ├── input.rs        # NotesInput, MeetingMetadata, LanguageCode
│   ├── dialect.rs      # MarkdownDialect + dialect cheat-sheets
│   ├── pipeline.rs     # NotesGenerator + stages
│   ├── prompts.rs      # prompt builders (system + user templates)
│   └── error.rs        # NotesError
├── exporter.rs         # NotesExporter trait, ExportArtifact, ExportError
└── conformance/
    ├── llm.rs          # LlmProvider conformance harness
    ├── notes.rs        # NotesGenerator end-to-end harness
    ├── exporter.rs     # NotesExporter harness (golden-file)
    └── fakes.rs        # add MockLlm

crates/panops-portable/src/
├── lib.rs              # pub use new modules
├── genai_llm.rs        # GenaiLlm wrapping rust-genai
└── markdown_exporter.rs # MarkdownExporter

crates/panops-portable/tests/
├── conformance_genai_llm.rs       # gated behind PANOPS_RUN_LLM_TESTS env var
└── conformance_markdown_exporter.rs

crates/panops-engine/src/
└── main.rs              # extend to add `notes` subcommand

tests/fixtures/notes/
├── multi_speaker_60s.expected.md       # golden file
└── multi_speaker_60s.expected.json     # golden IR
```

## CLI surface (panops-engine)

Slice 04 adds a `notes` subcommand alongside the existing transcribe behavior:

```bash
panops notes <audio.wav> [--screenshots <dir>] [--out <dir>] [--dialect notion-enhanced|basic] [--no-diarize] [--llm-provider auto|ollama|openai|anthropic] [--llm-model <name>]
```

- `--out` defaults to `./<basename>-notes/`. Created if missing.
- `--screenshots` defaults to none → no images in output (notes still generated).
- `--dialect` defaults to `notion-enhanced`.
- `--llm-provider auto`: rust-genai's auto-detection (env vars for cloud keys, then `OLLAMA_HOST`).
- Existing `panops <audio.wav>` (no subcommand) keeps emitting JSON segments, unchanged.

Default `--llm-model` per provider:
- `ollama` → `gemma3:4b` (small, fast, local; matches "fast LLMs prompted differently")
- `anthropic` → `claude-haiku-4-5-20251001`
- `openai` → `gpt-4o-mini`
- `auto` resolves to whichever provider was detected, with the above per-provider default.

## Testing strategy

### Unit tests (panops-core)

- `notes::ir`: `Serialize` / `Deserialize` round-trip on every IR type.
- `notes::dialect`: dialect cheat-sheet strings exist, are non-empty, contain expected syntax markers.
- `notes::pipeline::topic_segmentation`: hand-built `Vec<Segment>` produces expected sections (gap rule, speaker-shift rule, min-length merging).
- `notes::pipeline::screenshot_anchoring`: screenshots with various ms_since_start placed in correct sections.

### Conformance harness

- `LlmProvider`: 3 tests
  1. `complete` with `Text` schema returns non-empty string.
  2. `complete` with `Json` schema returns valid JSON matching the schema.
  3. `complete` with malformed schema-required output errors with `LlmError::InvalidSchema`.

- `NotesGenerator` (end-to-end with `MockLlm`):
  1. Given the slice 03 fixture transcript (`multi_speaker_60s.json`) + 12 screenshot fixtures + canned LLM responses, produce `StructuredNotes` matching `tests/fixtures/notes/multi_speaker_60s.expected.json` (bit-exact except for `generated_at`).

- `NotesExporter`:
  1. Given the canned `StructuredNotes`, produce markdown bit-exactly matching `tests/fixtures/notes/multi_speaker_60s.expected.md` (frontmatter normalised: `panops_version: "TESTING"`, `generated_at: "2026-05-01T00:00:00Z"`).
  2. Both dialects produce different output (`NotionEnhanced` contains a `<callout` token; `Basic` does not).

### Real-LLM integration test (gated)

`crates/panops-portable/tests/conformance_genai_llm.rs` runs against a real provider. Skipped unless `PANOPS_RUN_LLM_TESTS=1` and at least one of `OLLAMA_HOST` / `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` is set. CI does not run this; documented in tests/fixtures/README.md.

### Property tests (deferred)

Topic segmentation invariants (sections cover the full transcript without gaps or overlaps; section boundaries fall on segment boundaries) → slice 04.5.

## Acceptance criteria

A slice is done when:

1. `cargo test --workspace --locked` passes on macOS (linux compile-only, per slice 02).
2. `cargo clippy --workspace --all-targets --locked -- -D warnings` clean.
3. `cargo run -p panops-engine -- notes tests/fixtures/audio/multi_speaker_60s.wav --screenshots tests/fixtures/screenshots --out /tmp/notes-test --dialect notion-enhanced --llm-provider ollama --llm-model gemma3:4b` produces a `notes.md` file (manual smoke; CI uses `MockLlm`).
4. Conformance harness with `MockLlm` produces bit-exact match against `tests/fixtures/notes/multi_speaker_60s.expected.md`.
5. Both `NotionEnhanced` and `Basic` outputs differ in expected ways.
6. `--no-diarize` flag still works in `notes` mode (sections collapse to 1 if no speaker info).
7. Frontmatter is valid YAML; `serde_yaml::from_str` round-trips.

## Out of scope (deferred to later slices)

- Speaker-name resolution (slice 04.5).
- Verifier / grounding pass that checks attributions against transcript (slice 04.5).
- Slack summary section (slice 04.5).
- Transcription corrections table (slice 04.5).
- LLM-driven topic clustering (slice 04.5).
- Cross-meeting context lookup (slice 08+; needs the cross-meeting registry from slice 5/6).
- Blank/low-info screenshot filtering (slice 04.5).
- Apple FoundationModels sidecar (slice 06; mac-shell slice).
- Notion exporter (post-v0.1).
- Output-language override (slice 04.5).
- Ensemble voting across multiple small models (slice 04.5+).
- Live notes streaming (slice 07).

## Risks

1. **Prompt quality**. Slice 04 baseline prompts will produce shallow notes. Acceptable: this slice locks architecture, not quality. Quality work fits cleanly on top.
2. **rust-genai version drift**. The `rust-genai` crate is pre-1.0 (0.4 as of 2026-05-01). Pin tightly; the adapter is small, churn cost is low.
3. **Local model availability**. Slice 04 manual smoke assumes Ollama is running with `gemma3:4b`. If it isn't, the smoke test fails. CI is unaffected (uses `MockLlm`). Documented in fixture README.
4. **Schema-constrained JSON not universally supported**. Some `rust-genai` providers (OpenAI, Anthropic) support structured-output / JSON mode; Ollama supports `format: json`. The adapter should request structured output when possible and fall back to "extract JSON from text" parsing when not. Adapter handles it; pipeline sees a clean `LlmResponse::Json`.
5. **Long meetings blow context window**. Slice 04 doesn't chunk per-section calls beyond what topic segmentation produces. A 90-minute single-section meeting (no gaps) would exceed 8k context on small models. Acceptable for slice 04 (fixtures are 60s); revisit when real meetings come in.

## Decisions made this slice

- **Structured-IR pipeline over single mega-prompt**: documented above.
- **`MockLlm` is the conformance fake, not a separate `FakeLlm` trait impl**: prompt-fingerprint table = simplest deterministic mock that catches prompt drift loudly.
- **Topic segmentation is rule-based in slice 04**: gap + speaker-shift + min-length. No LLM calls. Cheap, fast, good enough for the fixture.
- **Render is deterministic**: LLM produces narrative_md per section; the file structure (frontmatter, headings, separators, screenshot embeds) is Rust-emitted. This bounds LLM output and makes goldens stable.
- **Screenshot embedding is "append at section end" in slice 04**: not inline-by-paragraph. Inline placement is a prompt-engineering problem better solved when we have real meeting data.
- **`NotesGenerator` is concrete, not a trait**: only one impl planned; YAGNI on the trait.
- **Real-LLM tests are gated, not run in CI**: cost + flakiness; manual smoke only.
- **Default Ollama model = `gemma3:4b`**: small, fast, runs on any laptop with 8GB+ RAM, matches "fast LLMs prompted differently" intuition. Users can override.

## Open questions

None blocking slice 04. Tracked for future slices:
- How to surface LLM cost / latency to the user (slice 06 UI concern)?
- Cross-meeting context: keyword lookup vs. embedding similarity vs. LLM-driven retrieval (slice 08+)?
- When to trigger notes regeneration (after each meeting, on-demand, both)?
