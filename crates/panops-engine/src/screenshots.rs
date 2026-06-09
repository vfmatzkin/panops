//! Screenshot collection shared by the CLI `notes` subcommand and the
//! IPC `notes.generate` pipeline.
//!
//! Both paths need the same primitive: "read screenshot files from a
//! directory on disk, sort them, assign evenly-spaced timestamps across
//! the meeting's duration." Keeping it in one place prevents the IPC
//! path from silently diverging (which it did for a while — hardcoded
//! `Vec::new()` meant the app's notes never carried screenshots even
//! when the meeting had captured plenty).

use std::path::{Path, PathBuf};

use panops_core::notes::ir::Screenshot;

/// Read screenshot files from `dir`, sort lexicographically, and
/// assign evenly-spaced timestamps across `duration_ms`. Returns an
/// empty vec when `dir` does not exist or is empty — callers that
/// need to treat "user pointed at a missing dir" as an error (the
/// CLI's explicit `--screenshots` flag) check `dir.exists()`
/// themselves before calling.
///
/// Why empty-on-missing: the IPC path calls this for every
/// `notes.generate` against a meeting dir, and a freshly-started
/// meeting that hasn't captured any frames yet has no `screenshots/`
/// contents. Treating that as an error would fail every live-capture
/// notes generation that races ahead of the first screenshot.
pub fn collect_screenshots(dir: &Path, duration_ms: u64) -> Vec<Screenshot> {
    let entries = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => {
            // Other read_dir failures (permissions, FS flap) are
            // unexpected. Log + return empty rather than aborting the
            // whole notes pipeline — the notes are still useful
            // without screenshots, and the operator gets a trace line
            // to act on.
            tracing::warn!(error = %e, dir = ?dir, "collect_screenshots: read_dir failed");
            return Vec::new();
        }
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|r| r.ok().map(|e| e.path()))
        .filter(|p| p.is_file())
        .collect();
    files.sort();
    if files.is_empty() {
        return Vec::new();
    }
    let n = files.len() as u64;
    // Integer division: the last screenshot's timestamp lands just
    // short of `duration_ms`. With one screenshot, step = duration_ms
    // and its ts = 0 (the only frame anchors the meeting's start).
    let step = duration_ms.checked_div(n).unwrap_or(0);
    files
        .into_iter()
        .enumerate()
        .map(|(i, path)| Screenshot {
            ms_since_start: (i as u64) * step,
            path,
            caption: None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn missing_dir_returns_empty() {
        let got = collect_screenshots(Path::new("/nonexistent/path/that/does/not/exist"), 60_000);
        assert!(
            got.is_empty(),
            "missing dir must yield empty vec, not error"
        );
    }

    #[test]
    fn empty_dir_returns_empty() {
        let dir = tempfile::tempdir().expect("tempdir");
        let got = collect_screenshots(dir.path(), 60_000);
        assert!(got.is_empty());
    }

    #[test]
    fn reads_sorts_and_timestamps() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Create out-of-order so the sort is actually exercised.
        for name in ["003.jpg", "001.jpg", "002.jpg"] {
            fs::write(dir.path().join(name), b"fake-jpeg").expect("write fixture");
        }

        let got = collect_screenshots(dir.path(), 60_000);

        assert_eq!(got.len(), 3);
        // Lexicographic order: 001, 002, 003.
        assert!(got[0].path.ends_with("001.jpg"));
        assert!(got[1].path.ends_with("002.jpg"));
        assert!(got[2].path.ends_with("003.jpg"));
        // 60_000 / 3 = 20_000 step. Timestamps: 0, 20_000, 40_000.
        assert_eq!(got[0].ms_since_start, 0);
        assert_eq!(got[1].ms_since_start, 20_000);
        assert_eq!(got[2].ms_since_start, 40_000);
        // Caption stays unset — the collector has no model to caption with.
        for s in &got {
            assert!(s.caption.is_none());
        }
    }

    #[test]
    fn directories_inside_the_screenshots_folder_are_skipped() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join("001.jpg"), b"fake").expect("write");
        fs::create_dir(dir.path().join("subdir")).expect("mkdir");

        let got = collect_screenshots(dir.path(), 60_000);

        assert_eq!(got.len(), 1, "subdirs must not appear as screenshots");
        assert!(got[0].path.ends_with("001.jpg"));
    }

    #[test]
    fn single_screenshot_gets_zero_timestamp() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join("only.jpg"), b"fake").expect("write");

        let got = collect_screenshots(dir.path(), 60_000);

        assert_eq!(got.len(), 1);
        // 60_000 / 1 = 60_000 step, but the only frame's index is 0.
        assert_eq!(got[0].ms_since_start, 0);
    }

    #[test]
    fn zero_duration_does_not_panic() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join("001.jpg"), b"fake").expect("write");

        let got = collect_screenshots(dir.path(), 0);

        // checked_div(0, 1) = Some(0); no panic, timestamps all 0.
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].ms_since_start, 0);
    }
}
