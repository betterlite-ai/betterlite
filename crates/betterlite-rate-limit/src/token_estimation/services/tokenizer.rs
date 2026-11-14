use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_tokenizer::count::Counter;
use betterlite_tokenizer::count::request::{Message, TextInput, TokenRequest, ToolChoice};

use super::{InputTokenCounter, TokenCounterRequest};

#[async_trait]
impl InputTokenCounter for Counter {
    async fn count_tokens(&self, request: &TokenCounterRequest<'_>) -> Option<i64> {
        let messages = match request.messages {
            Some(messages) => Some(decode_messages(messages)?),
            None => None,
        };
        let text = request.text.and_then(text_input);
        let choices: Option<ToolChoice> = match request.tool_choice {
            Some(choice) => serde_json::from_value(serde_json::to_value(choice).ok()?).ok()?,
            None => None,
        };
        let counter_request = TokenRequest {
            model: request.model,
            custom_tokenizer: None,
            text: text.as_ref(),
            messages: messages.as_deref(),
            count_response_tokens: Some(false),
            tools: request.tools,
            tool_choice: choices.as_ref(),
        };
        let count = self.count(&counter_request).await.ok()?;
        i64::try_from(count).ok()
    }
}

fn decode_messages(messages: &[Json]) -> Option<Vec<Message>> {
    let mut decoded = Vec::with_capacity(messages.len());
    for message in messages {
        let value = serde_json::to_value(message).ok()?;
        decoded.push(serde_json::from_value(value).ok()?);
    }
    Some(decoded)
}

fn text_input(text: &Json) -> Option<TextInput> {
    match text {
        Json::Text(value) => Some(TextInput::One(value.clone())),
        Json::List(items) => {
            let mut parts = Vec::with_capacity(items.len());
            for item in items {
                let Json::Text(part) = item else {
                    return None;
                };
                parts.push(part.clone());
            }
            Some(TextInput::Many(parts))
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_) => None,
    }
}
