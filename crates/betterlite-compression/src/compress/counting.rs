use async_trait::async_trait;
use betterlite_tokenizer::count::Counter;
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::{Message, TextInput, TokenRequest};
use serde_json::Value;

#[async_trait]
pub trait TokenCounter: Send + Sync {
    async fn count_text(&self, model: &str, text: &str) -> Result<usize, CountError>;

    async fn count_messages(&self, model: &str, messages: &[Value]) -> Result<usize, CountError>;
}

#[async_trait]
impl TokenCounter for Counter {
    async fn count_text(&self, model: &str, text: &str) -> Result<usize, CountError> {
        let input = TextInput::One(text.to_owned());
        let request = TokenRequest {
            model,
            custom_tokenizer: None,
            text: Some(&input),
            messages: None,
            count_response_tokens: Some(false),
            tools: None,
            tool_choice: None,
        };
        self.count(&request).await
    }

    async fn count_messages(&self, model: &str, messages: &[Value]) -> Result<usize, CountError> {
        let parsed: Vec<Message> = serde_json::from_value(Value::Array(messages.to_vec()))
            .map_err(|source| CountError::Json { source })?;
        let request = TokenRequest {
            model,
            custom_tokenizer: None,
            text: None,
            messages: Some(&parsed),
            count_response_tokens: Some(false),
            tools: None,
            tool_choice: None,
        };
        self.count(&request).await
    }
}

mod tests;
