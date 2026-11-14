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
        self.count(&token_request(model, Some(&input), None)).await
    }

    async fn count_messages(&self, model: &str, messages: &[Value]) -> Result<usize, CountError> {
        let parsed: Vec<Message> = serde_json::from_value(Value::Array(messages.to_vec()))
            .map_err(|source| CountError::Json { source })?;
        self.count(&token_request(model, None, Some(&parsed))).await
    }
}

const fn token_request<'request>(
    model: &'request str,
    text: Option<&'request TextInput>,
    messages: Option<&'request [Message]>,
) -> TokenRequest<'request> {
    TokenRequest {
        model,
        custom_tokenizer: None,
        text,
        messages,
        count_response_tokens: Some(false),
        tools: None,
        tool_choice: None,
    }
}

mod tests;
