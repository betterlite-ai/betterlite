use std::sync::Arc;

use betterlite_models::decisions::Json;
use betterlite_tokenizer::count::Counter;
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::{Message, TokenRequest};
use tokio::runtime::Handle;

use crate::api::messages_context_clear::error::ClearToolUsesError;

#[derive(Debug, Clone, Copy)]
pub struct TokenCountRequest<'request> {
    pub model: &'request str,
    pub messages: &'request [Json],
    pub tools: Option<&'request [Json]>,
}

pub trait TokenCounter {
    fn count(&self, request: TokenCountRequest<'_>) -> Result<usize, ClearToolUsesError>;
}

pub struct TokenizerCounter {
    pub counter: Arc<Counter>,
    pub runtime: Handle,
}

impl TokenCounter for TokenizerCounter {
    fn count(&self, request: TokenCountRequest<'_>) -> Result<usize, ClearToolUsesError> {
        if Handle::try_current().is_ok() {
            return Err(ClearToolUsesError::BlockingRuntime);
        }
        let messages = decode_messages(request.messages)?;
        let parsed = TokenRequest {
            model: request.model,
            custom_tokenizer: None,
            text: None,
            messages: Some(&messages),
            count_response_tokens: Some(false),
            tools: request.tools,
            tool_choice: None,
        };
        self.runtime
            .block_on(self.counter.count(&parsed))
            .map_err(ClearToolUsesError::TokenCount)
    }
}

fn decode_messages(messages: &[Json]) -> Result<Vec<Message>, ClearToolUsesError> {
    let mut decoded = Vec::with_capacity(messages.len());
    for message in messages {
        let value = serde_json::to_value(message)
            .map_err(|source| ClearToolUsesError::TokenCount(CountError::Json { source }))?;
        decoded.push(
            serde_json::from_value(value)
                .map_err(|source| ClearToolUsesError::TokenCount(CountError::Json { source }))?,
        );
    }
    Ok(decoded)
}
