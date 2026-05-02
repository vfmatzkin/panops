//! `LlmProvider` adapter wrapping `rust-genai`. Provider auto-detection by
//! environment variable. Synchronous trait calls block on a private tokio
//! runtime.

use std::sync::Arc;

use panops_core::llm::{LlmError, LlmProvider, LlmRequest, LlmResponse};
use tokio::runtime::Runtime;

pub struct GenaiLlm {
    client: genai::Client,
    model: String,
    rt: Arc<Runtime>,
}

impl GenaiLlm {
    pub fn new(model: impl Into<String>) -> Result<Self, LlmError> {
        let rt = Runtime::new().map_err(|e| LlmError::Provider(e.to_string()))?;
        Ok(Self {
            // reason: Client::default() reads provider keys from env automatically
            client: genai::Client::default(),
            model: model.into(),
            rt: Arc::new(rt),
        })
    }

    /// Auto-detect provider and pick a sensible default model for it.
    pub fn auto() -> Result<Self, LlmError> {
        let model = if std::env::var("ANTHROPIC_API_KEY").is_ok() {
            "claude-haiku-4-5-20251001"
        } else if std::env::var("OPENAI_API_KEY").is_ok() {
            "gpt-4o-mini"
        } else if std::env::var("OLLAMA_HOST").is_ok() {
            "gemma3:4b"
        } else {
            return Err(LlmError::Provider(
                "no provider configured; set OLLAMA_HOST, ANTHROPIC_API_KEY, or OPENAI_API_KEY"
                    .into(),
            ));
        };
        Self::new(model)
    }
}

impl LlmProvider for GenaiLlm {
    fn complete(&self, req: LlmRequest) -> Result<LlmResponse, LlmError> {
        use genai::chat::{ChatMessage, ChatOptions, ChatRequest};

        let mut messages: Vec<ChatMessage> = Vec::new();
        if let Some(sys) = req.system.clone() {
            messages.push(ChatMessage::system(sys));
        }
        messages.push(ChatMessage::user(req.user.clone()));

        let chat_req = ChatRequest::new(messages);

        let options = ChatOptions::default()
            .with_temperature(req.temperature as f64)
            .with_max_tokens(req.max_tokens);

        let client = self.client.clone();
        let model = self.model.clone();
        let resp = self.rt.block_on(async move {
            client
                .exec_chat(&model, chat_req, Some(&options))
                .await
                .map_err(|e| LlmError::Provider(e.to_string()))
        })?;

        // reason: first_text() returns Option<&str>; into_first_text() gives owned String
        let text = resp
            .first_text()
            .ok_or(LlmError::EmptyResponse)?
            .to_string();

        if req.schema.is_some() {
            // We do NOT request `ChatResponseFormat::JsonMode`. Tested via
            // genai 0.5.3 against Ollama's OpenAI-compat `/v1/chat/completions`
            // endpoint with gemma3:4b: JsonMode causes the model to return an
            // empty `{}` regardless of prompt. The native Ollama `/api/chat`
            // endpoint honors `format: "json"` correctly, but genai doesn't
            // route there. Until that lands upstream, we let the model emit
            // its natural fenced output and strip the fences.
            let json_text = strip_markdown_fences(&text);
            match serde_json::from_str::<serde_json::Value>(json_text) {
                Ok(v) => Ok(LlmResponse::Json(v)),
                Err(e) => Err(LlmError::InvalidSchema {
                    expected: "json object".into(),
                    got: format!("text ({e})"),
                }),
            }
        } else {
            Ok(LlmResponse::Text(text))
        }
    }
}

fn strip_markdown_fences(s: &str) -> &str {
    let s = s.trim();
    let Some(inner) = s.strip_prefix("```json").or_else(|| s.strip_prefix("```")) else {
        return s;
    };
    let trimmed = inner.trim();
    // Use strip_suffix (single match) rather than trim_end_matches (greedy) so
    // a JSON value containing literal triple-backticks isn't corrupted.
    trimmed.strip_suffix("```").unwrap_or(trimmed).trim()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_fences_passes_through_bare_json() {
        assert_eq!(strip_markdown_fences("{\"k\":1}"), "{\"k\":1}");
        assert_eq!(strip_markdown_fences("  {\"k\":1}  "), "{\"k\":1}");
    }

    #[test]
    fn strip_fences_unwraps_json_tagged_block() {
        let input = "```json\n{\"k\":1}\n```";
        assert_eq!(strip_markdown_fences(input), "{\"k\":1}");
    }

    #[test]
    fn strip_fences_unwraps_plain_block() {
        let input = "```\n{\"k\":1}\n```";
        assert_eq!(strip_markdown_fences(input), "{\"k\":1}");
    }

    #[test]
    fn strip_fences_handles_trailing_whitespace_inside_block() {
        let input = "  ```json\n{\"k\":1}\n```  ";
        assert_eq!(strip_markdown_fences(input), "{\"k\":1}");
    }

    #[test]
    fn strip_fences_does_not_eat_literal_backticks_inside_json_strings() {
        // Bare JSON whose value contains a triple-backtick token must round-trip.
        let input = "{\"text\":\"```code```\"}";
        assert_eq!(strip_markdown_fences(input), "{\"text\":\"```code```\"}");
    }

    #[test]
    fn strip_fences_only_strips_one_trailing_fence_inside_block() {
        // A fenced block whose JSON value itself contains ```. The closing
        // fence is removed once; the JSON content is preserved.
        let input = "```json\n{\"text\":\"```code```\"}\n```";
        assert_eq!(strip_markdown_fences(input), "{\"text\":\"```code```\"}");
    }
}
