//! Additive raw-transcript sidecar.
//!
//! The notes pipeline feeds raw ASR segments through an LLM that rewrites and
//! translates content, so `notes.md` is a synthesis, not a faithful record of
//! what Whisper heard. This module renders the raw segments to a
//! human-readable, grep-friendly `transcript.txt` written *alongside*
//! `notes.md`. It is deliberately **not** part of the [`NotesExporter`] contract
//! — the sidecar is an extra source-of-truth artifact, never a replacement, so
//! users can compare the synthesized notes against the original transcript.
//!
//! [`NotesExporter`]: crate::exporter::NotesExporter

use std::io;
use std::path::{Path, PathBuf};

use crate::Segment;

/// Filename of the raw-transcript sidecar, written next to `notes.md`.
pub(crate) const RAW_TRANSCRIPT_FILENAME: &str = "transcript.txt";

/// Render raw ASR segments to a grep-friendly transcript body.
///
/// One line per segment:
/// `[m:ss.mmm-m:ss.mmm] <speaker_id> (<lang>): <text>`
///
/// - Timestamps carry millisecond precision so `start_ms`/`end_ms` survive the
///   round-trip (the human-readable `m:ss.mmm` form still reads cleanly).
/// - `<speaker_id>` is `speaker_N` when diarized, else `unknown`.
/// - `<lang>` is the detected language code, else `und` (undetermined).
/// - `<text>` is trimmed and newline-collapsed so every segment stays on one
///   line — the exact, untrimmed text remains in the sibling `transcript.json`.
pub fn render_raw_transcript(segments: &[Segment]) -> String {
    let mut out = String::new();
    for seg in segments {
        let speaker = match seg.speaker_id {
            Some(id) => format!("speaker_{id}"),
            None => "unknown".to_string(),
        };
        let lang = seg.language_detected.as_deref().unwrap_or("und");
        let text = seg.text.replace(['\n', '\r'], " ");
        out.push_str(&format!(
            "[{}-{}] {speaker} ({lang}): {}\n",
            format_ts(seg.start_ms),
            format_ts(seg.end_ms),
            text.trim(),
        ));
    }
    out
}

/// Format a millisecond timestamp as `m:ss.mmm` (minutes unpadded, human-readable;
/// the `.mmm` milliseconds keep the value lossless / round-trippable).
fn format_ts(ms: u64) -> String {
    let total_s = ms / 1000;
    let millis = ms % 1000;
    let m = total_s / 60;
    let s = total_s % 60;
    format!("{m}:{s:02}.{millis:03}")
}

/// Write the raw-transcript sidecar into `dest`, returning the written path.
///
/// `dest` is the notes output directory (the same dir `notes.md` lands in).
/// This is additive and best-effort at the call sites: a failure here must not
/// abort notes generation.
pub fn write_raw_transcript(segments: &[Segment], dest: &Path) -> io::Result<PathBuf> {
    let path = dest.join(RAW_TRANSCRIPT_FILENAME);
    std::fs::write(&path, render_raw_transcript(segments))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_segments() -> Vec<Segment> {
        vec![
            Segment {
                start_ms: 0,
                end_ms: 4_500,
                text: "Hello, everyone.".into(),
                language_detected: Some("en".into()),
                confidence: 0.95,
                speaker_id: Some(0),
            },
            Segment {
                start_ms: 4_500,
                end_ms: 9_200,
                text: "  Hola a todos.  ".into(),
                language_detected: Some("es".into()),
                confidence: 0.91,
                speaker_id: Some(1),
            },
            Segment {
                start_ms: 65_000,
                end_ms: 70_000,
                text: "Crosses a\nline break.".into(),
                language_detected: None,
                confidence: 0.5,
                speaker_id: None,
            },
        ]
    }

    const EXPECTED: &str = "\
[0:00.000-0:04.500] speaker_0 (en): Hello, everyone.
[0:04.500-0:09.200] speaker_1 (es): Hola a todos.
[1:05.000-1:10.000] unknown (und): Crosses a line break.
";

    #[test]
    fn render_emits_one_line_per_segment_with_all_fields() {
        assert_eq!(render_raw_transcript(&fixture_segments()), EXPECTED);
    }

    #[test]
    fn render_empty_transcript_is_empty_string() {
        assert_eq!(render_raw_transcript(&[]), "");
    }

    #[test]
    fn write_creates_named_file_with_rendered_body() {
        let tmp = tempdir();
        let segs = fixture_segments();
        let path = write_raw_transcript(&segs, tmp.path()).expect("write should succeed");

        assert_eq!(path, tmp.path().join(RAW_TRANSCRIPT_FILENAME));
        assert!(path.exists(), "sidecar should exist on disk: {path:?}");
        let body = std::fs::read_to_string(&path).expect("read back sidecar");
        assert_eq!(body, EXPECTED);
    }

    /// std-only tempdir, mirroring the exporter conformance harness — this
    /// crate intentionally has no `tempfile` dependency.
    fn tempdir() -> TempDirHandle {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("panops-raw-transcript-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&path).expect("create tempdir");
        TempDirHandle { path }
    }

    struct TempDirHandle {
        path: PathBuf,
    }

    impl TempDirHandle {
        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDirHandle {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}
