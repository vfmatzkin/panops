use thiserror::Error;

#[derive(Debug, Error)]
pub enum NotesError {
    #[error("empty transcript")]
    EmptyTranscript,
    #[error("llm: {0}")]
    Llm(#[from] crate::llm::LlmError),
    /// Every per-section narrative LLM call returned a provider error, so the
    /// model is unavailable at runtime. Returning notes here would write an
    /// all-default, empty-section file (every section a `## N. Section` stub),
    /// which is worthless — so the pipeline fails instead of reporting success.
    #[error(
        "notes LLM unavailable: all {failed} of {total} section generation calls failed ({last_error})"
    )]
    LlmUnavailable {
        failed: usize,
        total: usize,
        last_error: String,
    },
    #[error("schema mismatch in stage {stage}: {detail}")]
    SchemaMismatch { stage: &'static str, detail: String },
    #[error("invalid input: {0}")]
    InvalidInput(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_error_display_includes_variant_name() {
        let e = NotesError::EmptyTranscript;
        assert!(format!("{e}").contains("empty"));
    }
}
