use async_trait::async_trait;
use betterlite_openai::chat::response::ChatResponse;
use betterlite_openai::embedding::EmbeddingResponse;
use betterlite_openai::gigachat_chat::config::GigachatBearer;
use betterlite_openai::gigachat_chat::error::GigaChatError;
use betterlite_openai::gigachat_passthrough::GigachatPassthroughError;
use betterlite_openai::passthrough::transformation::types::{PassthroughLogging, RelayCallType};
use betterlite_streaming::stream_chunk_shaping::records::{GenericChunk, Usage};
use secrecy::SecretString;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::error::HelperError;
use tokio::runtime::{Builder, Runtime};

pub(crate) struct TokenBearer {
    pub(crate) token: Option<SecretString>,
}

#[async_trait]
impl GigachatBearer for TokenBearer {
    async fn access_token(
        &self,
        _credentials: Option<&SecretString>,
    ) -> Result<SecretString, GigaChatError> {
        self.token.as_ref().map_or_else(
            || {
                Err(GigaChatError {
                    status_code: 401,
                    message: String::from("GigaChat credentials not provided."),
                    headers: None,
                    source: None,
                })
            },
            |token| Ok(token.clone()),
        )
    }
}

pub(crate) struct NoopLogging;

impl PassthroughLogging for NoopLogging {
    fn set_call_type(&mut self, _call_type: RelayCallType) {}
}

pub(crate) fn runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread().build().map_err(|error| {
        Box::new(HelperError {
            class: String::from("Runtime"),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })
}

pub(crate) fn error_status(error: &GigachatPassthroughError) -> Value {
    match error {
        GigachatPassthroughError::Token(source) => json!({"raised": source.status_code}),
        GigachatPassthroughError::Passthrough(_) | GigachatPassthroughError::Provider(_) => {
            json!({"raised": null})
        }
    }
}

pub(crate) fn chat_value(reply: &ChatResponse) -> Value {
    let choices: Vec<Value> = reply
        .choices
        .iter()
        .map(|choice| {
            json!({
                "index": choice.index,
                "finish_reason": choice.finish_reason,
                "message": {
                    "role": choice.message.role,
                    "content": choice.message.content,
                },
            })
        })
        .collect();
    json!({
        "id": reply.id,
        "created": reply.created,
        "model": reply.model,
        "choices": choices,
        "usage": {
            "prompt_tokens": reply.usage.prompt_tokens,
            "completion_tokens": reply.usage.completion_tokens,
            "total_tokens": reply.usage.total_tokens,
        },
    })
}

pub(crate) fn embedding_value(reply: &EmbeddingResponse) -> Value {
    json!({
        "object": reply.object,
        "model": reply.model,
        "data": reply.data,
        "usage": {
            "prompt_tokens": reply.usage.prompt_tokens,
            "total_tokens": reply.usage.total_tokens,
        },
    })
}

pub(crate) fn collected_value(chunks: &[GenericChunk]) -> Value {
    let mut text = String::new();
    let mut finish_reason: Option<&str> = None;
    let mut usage: Option<&Usage> = None;
    for chunk in chunks {
        text.push_str(&chunk.text);
        if !chunk.finish_reason.is_empty() {
            finish_reason = Some(chunk.finish_reason.as_str());
        }
        if let Some(present) = &chunk.usage {
            usage = Some(present);
        }
    }
    json!({
        "text": text,
        "finish_reason": finish_reason,
        "usage": usage.map(|present| json!({
            "prompt_tokens": present.prompt_tokens,
            "completion_tokens": present.completion_tokens,
            "total_tokens": present.total_tokens,
        })),
    })
}
