use async_trait::async_trait;
use betterlite_tokenizer::count::Counter;
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::{Message, TokenRequest};
use serde_json::Value;
use thiserror::Error;

#[async_trait]
pub trait PromptTokens: Send + Sync {
    async fn count(&self, model: &str, messages: &[Value]) -> Result<usize, PromptCountError>;
}

#[derive(Debug, Error)]
pub enum PromptCountError {
    #[error("the prompt messages are not chat messages: {source}")]
    Messages {
        #[source]
        source: serde_json::Error,
    },
    #[error("the prompt tokens could not be counted: {source}")]
    Count {
        #[from]
        source: CountError,
    },
}

#[async_trait]
impl PromptTokens for Counter {
    async fn count(&self, model: &str, messages: &[Value]) -> Result<usize, PromptCountError> {
        let parsed: Vec<Message> = serde_json::from_value(Value::Array(messages.to_vec()))
            .map_err(|source| PromptCountError::Messages { source })?;
        let request = TokenRequest {
            model,
            custom_tokenizer: None,
            text: None,
            messages: Some(&parsed),
            // LiteLLM omits this argument, whose False default includes the assistant reply priming.
            count_response_tokens: Some(false),
            tools: None,
            tool_choice: None,
        };
        Ok(Self::count(self, &request).await?)
    }
}
