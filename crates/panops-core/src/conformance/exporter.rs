//! Conformance harness for [`crate::exporter::NotesExporter`] adapters.
//!
//! Every exporter (real or fake) must pass the same suite. The harness
//! exercises the contract documented on the trait:
//! - `export(notes, dest)` writes to `dest` and returns an [`ExportArtifact`]
//!   whose `primary_file` exists, lives under `dest`, and is non-empty.
//! - Every path in `assets` exists and lives under `dest`.
//! - When `dest` exists but is a file (not a directory), the exporter must
//!   return [`ExportError::InvalidDest`] without overwriting it. This guards
//!   against silent data loss.
//!
//! Whether `dest` must already exist as a directory or may be auto-created
//! is an exporter-specific policy choice and intentionally NOT asserted here.

use std::path::{Path, PathBuf};

use crate::exporter::{ExportError, NotesExporter};
use crate::notes::dialect::MarkdownDialect;
use crate::notes::ir::{NotesFrontmatter, NotesSection, StructuredNotes};

/// Run the full conformance suite against an exporter implementation.
pub fn run_suite<E: NotesExporter>(exporter: &E) {
    happy_path_writes_primary_under_dest(exporter);
    assets_exist_and_live_under_dest(exporter);
    invalid_dest_when_dest_is_a_file(exporter);
}

fn happy_path_writes_primary_under_dest<E: NotesExporter>(exporter: &E) {
    let tmp = tempdir();
    let notes = sample_notes();
    let art = exporter
        .export(&notes, tmp.path())
        .expect("happy-path export should succeed");

    assert!(
        art.primary_file.exists(),
        "primary_file should exist on disk: {:?}",
        art.primary_file
    );
    assert!(
        art.primary_file.starts_with(tmp.path()),
        "primary_file {:?} must live under dest {:?}",
        art.primary_file,
        tmp.path()
    );
    let len = std::fs::metadata(&art.primary_file)
        .expect("stat primary_file")
        .len();
    assert!(
        len > 0,
        "primary_file should be non-empty (got {len} bytes)"
    );
}

fn assets_exist_and_live_under_dest<E: NotesExporter>(exporter: &E) {
    let tmp = tempdir();
    let art = exporter.export(&sample_notes(), tmp.path()).unwrap();
    for asset in &art.assets {
        assert!(asset.exists(), "asset {:?} should exist on disk", asset);
        assert!(
            asset.starts_with(tmp.path()),
            "asset {:?} must live under dest {:?}",
            asset,
            tmp.path()
        );
    }
}

fn invalid_dest_when_dest_is_a_file<E: NotesExporter>(exporter: &E) {
    let tmp = tempdir();
    let dest = tmp.path().join("is-a-file");
    std::fs::write(&dest, b"not a dir").unwrap();
    let err = exporter
        .export(&sample_notes(), &dest)
        .expect_err("export to file-instead-of-dir should fail");
    assert!(
        matches!(err, ExportError::InvalidDest(_)),
        "expected InvalidDest, got {err:?}"
    );
}

fn sample_notes() -> StructuredNotes {
    use chrono::{FixedOffset, TimeZone, Utc};
    StructuredNotes {
        schema_version: StructuredNotes::SCHEMA_VERSION,
        frontmatter: NotesFrontmatter {
            title: "Conformance sample".into(),
            date: chrono::NaiveDate::from_ymd_opt(2026, 5, 1).unwrap(),
            started_at: FixedOffset::east_opt(0)
                .unwrap()
                .with_ymd_and_hms(2026, 5, 1, 10, 0, 0)
                .unwrap(),
            duration_ms: 60_000,
            speakers: vec!["speaker_0".into()],
            tags: vec!["conformance".into()],
            template: "default".into(),
            dialect: MarkdownDialect::Basic,
            panops_version: "TEST".into(),
            source_audio: None,
        },
        sections: vec![NotesSection {
            index: 1,
            title: "Section A".into(),
            time_range_ms: (0, 60_000),
            narrative_md: "Some content.".into(),
            key_points: vec![],
            action_items: vec![],
            screenshots: vec![],
        }],
        language: "en".into(),
        generated_at: Utc.with_ymd_and_hms(2026, 5, 1, 10, 1, 0).unwrap(),
    }
}

/// Helper that mirrors what `tempfile::tempdir()` would give us, but built on
/// `std` only — `panops-core` does not (and should not) depend on `tempfile`
/// at non-dev-dep level. Adapters that pull in tempfile can use that directly
/// in their own tests; this harness needs to work from the conformance crate.
fn tempdir() -> TempDirHandle {
    let base = std::env::temp_dir();
    // Use a counter+pid to avoid collisions inside one process. We don't need
    // strong randomness — these are single-test working dirs.
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let path = base.join(format!(
        "panops-exporter-conformance-{}-{n}",
        std::process::id()
    ));
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
