use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    pub language_detected: Option<String>,
    pub confidence: f32,
    pub is_partial: bool,
    #[serde(default)]
    pub speaker_id: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transcript {
    pub schema_version: u32,
    pub model: String,
    pub audio_path: PathBuf,
    pub audio_duration_ms: u64,
    #[serde(default)]
    pub diarized: bool,
    pub segments: Vec<Segment>,
}

impl Transcript {
    pub const SCHEMA_VERSION: u32 = 2;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcript_roundtrips_through_json() {
        let t = Transcript {
            schema_version: Transcript::SCHEMA_VERSION,
            model: "ggml-tiny-q5_1".to_string(),
            audio_path: PathBuf::from("foo.wav"),
            audio_duration_ms: 30_000,
            diarized: true,
            segments: vec![Segment {
                start_ms: 0,
                end_ms: 4_500,
                text: "hello world".to_string(),
                language_detected: Some("en".to_string()),
                confidence: 0.91,
                is_partial: false,
                speaker_id: Some(0),
            }],
        };
        let json = serde_json::to_string(&t).unwrap();
        let back: Transcript = serde_json::from_str(&json).unwrap();
        assert_eq!(back.segments, t.segments);
        assert_eq!(back.schema_version, 2);
        assert!(back.diarized);
    }

    #[test]
    fn transcript_v1_payload_deserializes_with_defaults() {
        // Older v1 JSON without diarized/speaker_id should still parse.
        let json = r#"{
            "schema_version": 1,
            "model": "ggml-tiny-q5_1",
            "audio_path": "foo.wav",
            "audio_duration_ms": 30000,
            "segments": [{
                "start_ms": 0,
                "end_ms": 4500,
                "text": "hello",
                "language_detected": "en",
                "confidence": 0.9,
                "is_partial": false
            }]
        }"#;
        let t: Transcript = serde_json::from_str(json).unwrap();
        assert!(!t.diarized);
        assert_eq!(t.segments[0].speaker_id, None);
    }
}
