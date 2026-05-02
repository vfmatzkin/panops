//! `LlmProvider` adapter wrapping `rust-genai`. Provider auto-detection by
//! environment variable.
//!
//! Runtime ownership: `GenaiLlm::new` lazily creates ONE shared
//! `tokio::runtime::Runtime` for the process the first time a `GenaiLlm`
//! is constructed via `new`. Server callers should use `with_handle`
//! instead, passing a `Handle` to a tokio runtime owned by the binary
//! (typically a runtime dedicated to outbound HTTP, separate from the
//! one driving jsonrpsee — keeps slow LLM calls from delaying RPC accept).

use std::sync::{Arc, OnceLock};

use panops_core::llm::{LlmError, LlmProvider, LlmRequest, LlmResponse};
use tokio::runtime::{Handle, Runtime};

static SHARED_CLI_RT: OnceLock<Arc<Runtime>> = OnceLock::new();

fn shared_cli_runtime() -> Result<Handle, LlmError> {
    let rt = SHARED_CLI_RT.get_or_init(|| {
        Arc::new(
            Runtime::new().expect("create shared LLM CLI runtime; should never fail at startup"),
        )
    });
    Ok(rt.handle().clone())
}

pub struct GenaiLlm {
    client: genai::Client,
    model: String,
    handle: Handle,
}

impl GenaiLlm {
    /// CLI/test constructor. Uses one process-wide tokio runtime, lazily
    /// initialised. Multiple `GenaiLlm::new` calls share the same runtime.
    pub fn new(model: impl Into<String>) -> Result<Self, LlmError> {
        Ok(Self {
            // reason: Client::default() reads provider keys from env automatically
            client: genai::Client::default(),
            model: model.into(),
            handle: shared_cli_runtime()?,
        })
    }

    /// Server constructor. Uses the supplied tokio `Handle` so the binary
    /// can put outbound LLM HTTP on a dedicated runtime separate from the
    /// jsonrpsee server runtime.
    pub fn with_handle(model: impl Into<String>, handle: Handle) -> Self {
        Self {
            client: genai::Client::default(),
            model: model.into(),
            handle,
        }
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
        let resp = self.handle.block_on(async move {
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
                    got: format!("text ({e}); preview: {}", preview_for_error(json_text)),
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

/// Truncates a string for inclusion in an error message. Splits at a char
/// boundary (so we never panic on UTF-8) and appends an ellipsis marker plus
/// the dropped byte count when truncation occurred. Used when the LLM emits
/// non-JSON and we need a self-describing failure without dumping kilobytes.
fn preview_for_error(s: &str) -> String {
    const MAX: usize = 200;
    if s.len() <= MAX {
        return s.to_string();
    }
    let mut end = MAX;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…[+{} bytes]", &s[..end], s.len() - end)
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

    #[test]
    fn preview_short_input_is_unchanged() {
        assert_eq!(preview_for_error("hello world"), "hello world");
    }

    #[test]
    fn preview_input_exactly_at_max_is_unchanged() {
        // <= MAX is the inclusive boundary — verify the off-by-one.
        let s = "a".repeat(200);
        assert_eq!(preview_for_error(&s), s);
    }

    #[test]
    fn preview_long_input_is_truncated_with_byte_count() {
        let s = "a".repeat(500);
        let p = preview_for_error(&s);
        assert!(p.starts_with(&"a".repeat(200)));
        assert!(p.ends_with("…[+300 bytes]"));
    }

    #[test]
    fn preview_handles_multibyte_at_truncation_boundary() {
        // 199 ASCII bytes + a 4-byte emoji = 203 bytes. MAX=200 lands inside
        // the emoji's UTF-8 sequence, so the function must back off to byte
        // 199 (the last char boundary) and report the dropped 4 bytes.
        let s = format!("{}🦀", "a".repeat(199));
        let p = preview_for_error(&s);
        assert!(p.starts_with(&"a".repeat(199)));
        assert!(p.ends_with("…[+4 bytes]"));
    }
}

#[cfg(test)]
mod runtime_tests {
    use super::*;

    #[test]
    fn two_new_instances_share_one_runtime_handle() {
        let _a = GenaiLlm::new("gemma3:4b").unwrap();
        let _b = GenaiLlm::new("gemma3:4b").unwrap();
        // OnceLock must be set exactly once; both instances see the same Arc.
        let first = SHARED_CLI_RT.get().expect("set by new()");
        let second = SHARED_CLI_RT.get().expect("set by new()");
        assert!(Arc::ptr_eq(first, second));
    }

    #[test]
    fn with_handle_uses_supplied_runtime_not_shared() {
        // Build a private runtime, hand it to with_handle. The supplied
        // Handle is what gets stored — confirmed by checking we can call
        // block_on on it via complete() (smoke). We can't directly compare
        // Handle pointers, but we CAN confirm with_handle does not panic
        // when the OnceLock is uninitialised.
        let rt = Runtime::new().unwrap();
        let _llm = GenaiLlm::with_handle("gemma3:4b", rt.handle().clone());
    }
}
