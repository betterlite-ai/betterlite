use async_trait::async_trait;
use betterlite_tokenizer::count::Counter;
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::{Message, TextInput, TokenRequest};

#[async_trait]
pub trait TokenCounter: Send + Sync {
    async fn count_messages(&self, messages: &[Message]) -> Result<i64, CountError>;

    async fn count_text(&self, text: &str) -> Result<i64, CountError>;
}

#[async_trait]
impl TokenCounter for Counter {
    async fn count_messages(&self, messages: &[Message]) -> Result<i64, CountError> {
        let request = TokenRequest {
            model: "",
            custom_tokenizer: None,
            text: None,
            messages: Some(messages),
            count_response_tokens: Some(false),
            tools: None,
            tool_choice: None,
        };
        let total = Counter::count(self, &request).await?;
        i64::try_from(total).map_err(|_conversion| CountError::Overflow)
    }

    async fn count_text(&self, text: &str) -> Result<i64, CountError> {
        let input = TextInput::One(text.to_owned());
        let request = TokenRequest {
            model: "",
            custom_tokenizer: None,
            text: Some(&input),
            messages: None,
            count_response_tokens: Some(true),
            tools: None,
            tool_choice: None,
        };
        let total = Counter::count(self, &request).await?;
        i64::try_from(total).map_err(|_conversion| CountError::Overflow)
    }
}
