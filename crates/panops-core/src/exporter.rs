use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::notes::ir::StructuredNotes;

pub trait NotesExporter: Send + Sync {
    fn export(&self, notes: &StructuredNotes, dest: &Path) -> Result<ExportArtifact, ExportError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportArtifact {
    pub primary_file: PathBuf,
    pub assets: Vec<PathBuf>,
}

#[derive(Debug, Error)]
pub enum ExportError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid destination: {0}")]
    InvalidDest(String),
    #[error("render: {0}")]
    Render(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_error_display_includes_variant_name() {
        let e = ExportError::Io(std::io::Error::other("disk full"));
        assert!(format!("{e}").contains("io"));
    }

    #[test]
    fn export_artifact_can_be_constructed() {
        let a = ExportArtifact {
            primary_file: std::path::PathBuf::from("/tmp/notes.md"),
            assets: vec![std::path::PathBuf::from("/tmp/screenshots/1.jpg")],
        };
        assert_eq!(a.assets.len(), 1);
    }
}
