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
        count_request(self, None, Some(messages), false).await
    }

    async fn count_text(&self, text: &str) -> Result<i64, CountError> {
        let input = TextInput::One(text.to_owned());
        count_request(self, Some(&input), None, true).await
    }
}

async fn count_request(
    counter: &Counter,
    text: Option<&TextInput>,
    messages: Option<&[Message]>,
    count_response_tokens: bool,
) -> Result<i64, CountError> {
    let request = TokenRequest {
        model: "",
        custom_tokenizer: None,
        text,
        messages,
        count_response_tokens: Some(count_response_tokens),
        tools: None,
        tool_choice: None,
    };
    let total = Counter::count(counter, &request).await?;
    i64::try_from(total).map_err(|_conversion| CountError::Overflow)
}
