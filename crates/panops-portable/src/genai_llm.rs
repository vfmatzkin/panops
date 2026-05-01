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
        use genai::chat::{ChatMessage, ChatOptions, ChatRequest, ChatResponseFormat};

        let mut messages: Vec<ChatMessage> = Vec::new();
        if let Some(sys) = req.system.clone() {
            messages.push(ChatMessage::system(sys));
        }
        messages.push(ChatMessage::user(req.user.clone()));

        let chat_req = ChatRequest::new(messages);

        // reason: always forward temperature/max_tokens so the provider honours
        // the request parameters; additionally, when a schema is requested, set
        // JsonMode so real LLMs (e.g. gemma3 via Ollama) emit raw JSON instead
        // of markdown-fenced blocks.
        let mut opts = ChatOptions::default()
            .with_temperature(f64::from(req.temperature))
            .with_max_tokens(req.max_tokens);
        if req.schema.is_some() {
            opts = opts.with_response_format(ChatResponseFormat::JsonMode);
        }

        let client = self.client.clone();
        let model = self.model.clone();
        let resp = self.rt.block_on(async move {
            client
                .exec_chat(&model, chat_req, Some(&opts))
                .await
                .map_err(|e| LlmError::Provider(e.to_string()))
        })?;

        // reason: first_text() returns Option<&str>; into_first_text() gives owned String
        let text = resp
            .first_text()
            .ok_or(LlmError::EmptyResponse)?
            .to_string();

        if req.schema.is_some() {
            match serde_json::from_str::<serde_json::Value>(&text) {
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
