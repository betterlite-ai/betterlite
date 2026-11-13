use std::sync::Arc;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_tokenizer::count::Counter;
use betterlite_tokenizer::reservation::schedule::AccountPool;
use betterlite_tokenizer::reservation::{ParsedRequest, RequestBody, count_input_tokens};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IoInputField {
    Messages,
    Prompt,
    Input,
}

impl IoInputField {
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Messages => "messages",
            Self::Prompt => "prompt",
            Self::Input => "input",
        }
    }
}

#[async_trait]
pub trait InputTokenCounter: Send + Sync {
    async fn count_field(&self, model: &str, field: IoInputField, value: &Json) -> Option<usize>;
}

// the pinned async pre-call offloads the token count once the input is large, through the same bounded pool
pub struct TokenizerInputCounter {
    pub counter: Arc<Counter>,
    pub pool: Arc<dyn AccountPool>,
}

#[async_trait]
impl InputTokenCounter for TokenizerInputCounter {
    async fn count_field(&self, model: &str, field: IoInputField, value: &Json) -> Option<usize> {
        let mut body = RequestBody::new();
        body.insert(field.key().to_owned(), value.clone());
        let request = ParsedRequest {
            body: &body,
            raw_body: None,
        };
        let models = [model.to_owned()];
        count_input_tokens(&self.counter, request, &models, self.pool.as_ref())
            .await
            .get(model)
            .copied()
    }
}

// LiteLLM hands token_counter either messages or one text field; both at once raises, which the estimate swallows to 0
#[must_use]
pub async fn estimate_input_tokens(
    counter: &dyn InputTokenCounter,
    model: &str,
    request: &RequestBody,
) -> i64 {
    let messages = present(request.get("messages"));
    let text = present(request.get("prompt")).map_or_else(
        || present(request.get("input")).map(|input| (IoInputField::Input, input)),
        |prompt| Some((IoInputField::Prompt, prompt)),
    );
    if messages.is_some() && text.is_some() {
        return 0;
    }
    let tokens = match (messages, text) {
        (Some(message_value), _) => {
            counter
                .count_field(model, IoInputField::Messages, message_value)
                .await
        }
        (None, Some((field, value))) => counter.count_field(model, field, value).await,
        (None, None) => None,
    };
    tokens.map_or(0, |estimate| i64::try_from(estimate).unwrap_or(i64::MAX))
}

const fn present(value: Option<&Json>) -> Option<&Json> {
    match value {
        Some(Json::Null) | None => None,
        Some(found) => Some(found),
    }
}
