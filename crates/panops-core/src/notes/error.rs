use thiserror::Error;

#[derive(Debug, Error)]
pub enum NotesError {
    #[error("empty transcript")]
    EmptyTranscript,
    #[error("llm: {0}")]
    Llm(#[from] crate::llm::LlmError),
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
