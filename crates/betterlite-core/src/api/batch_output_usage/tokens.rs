use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_tokenizer::count::Counter;
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::{Message, TextInput, TokenRequest};

use super::decode::number_is_integer;
use super::error::{BatchOutputUsageError, BatchResult};

type TokenCounter = dyn BatchTokenCounter;

// A batch request's input tokens scale roughly with its serialized size, so this is a conservative per-row fallback.
pub const TOKEN_ESTIMATE_BYTES_PER_TOKEN: usize = 4;

#[must_use]
pub const fn estimate_batch_entry_tokens(raw_line: &[u8]) -> usize {
    let tokens = raw_line.len().div_euclid(TOKEN_ESTIMATE_BYTES_PER_TOKEN);
    if tokens == 0 {
        return 1;
    }
    tokens
}

// B013's local token counter is async; the batch input path injects it instead of reaching for a runtime.
#[async_trait]
pub trait BatchTokenCounter: Send + Sync {
    async fn count_tokens(&self, request: &TokenRequest<'_>) -> Result<usize, CountError>;
}

#[async_trait]
impl BatchTokenCounter for Counter {
    // No test builds B013's Counter, whose fetcher and selector need a network client; a mutant of this body never runs.
    #[cfg_attr(test, mutants::skip)]
    async fn count_tokens(&self, request: &TokenRequest<'_>) -> Result<usize, CountError> {
        self.count(request).await
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

pub async fn count_batch_entry_tokens(
    entry: &Json,
    model_name: Option<&str>,
    counter: &TokenCounter,
) -> BatchResult<usize> {
    let Json::Object(fields) = entry else {
        return Err(BatchOutputUsageError::NotAnEntry);
    };
    let body = match fields.get("body") {
        None | Some(Json::Null) => return Ok(0),
        Some(Json::Object(body)) => body,
        Some(_) => return Err(BatchOutputUsageError::NotAnObject { field: "body" }),
    };
    let model = match body.get("model") {
        None | Some(Json::Null) => model_name.unwrap_or_default(),
        Some(Json::Text(model)) => model.as_str(),
        Some(_) => return Err(BatchOutputUsageError::NotAString { field: "model" }),
    };
    if let Some(messages) = present(body.get("messages")) {
        return count_messages(model, messages, counter).await;
    }
    if let Some(prompt) = present(body.get("prompt")) {
        return count_prompt(model, prompt, counter).await;
    }
    if let Some(input) = present(body.get("input")) {
        return count_prompt(model, input, counter).await;
    }
    Ok(0)
}
fn present(value: Option<&Json>) -> Option<&Json> {
    let found = value?;
    if matches!(found, Json::Null) {
        return None;
    }
    Some(found)
}
async fn count_messages(
    model: &str,
    messages: &Json,
    counter: &TokenCounter,
) -> BatchResult<usize> {
    let value = serde_json::to_value(messages)
        .map_err(|source| BatchOutputUsageError::Messages { source })?;
    let decoded: Vec<Message> = serde_json::from_value(value)
        .map_err(|source| BatchOutputUsageError::Messages { source })?;
    let request = token_request(model, None, Some(&decoded));
    Ok(counter.count_tokens(&request).await?)
}
async fn count_prompt(model: &str, value: &Json, counter: &TokenCounter) -> BatchResult<usize> {
    match value {
        Json::Text(text) => count_text(model, text, counter).await,
        Json::List(items) => {
            let mut total = 0_usize;
            for item in items {
                total = add_tokens(total, count_prompt_item(model, item, counter).await?)?;
            }
            Ok(total)
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_) => Ok(0),
    }
}

async fn count_prompt_item(model: &str, item: &Json, counter: &TokenCounter) -> BatchResult<usize> {
    match item {
        Json::Text(text) => count_text(model, text, counter).await,
        Json::Number(number) if number_is_integer(number) => Ok(1),
        Json::Bool(_) => Ok(1),
        Json::List(items) => {
            let mut total = 0_usize;
            for chunk in items {
                let contribution = match chunk {
                    Json::Text(text) => count_text(model, text, counter).await?,
                    Json::Number(number) if number_is_integer(number) => 1,
                    Json::Bool(_) => 1,
                    Json::Null | Json::Number(_) | Json::List(_) | Json::Object(_) => 0,
                };
                total = add_tokens(total, contribution)?;
            }
            Ok(total)
        }
        Json::Null | Json::Number(_) | Json::Object(_) => Ok(0),
    }
}

async fn count_text(model: &str, text: &str, counter: &TokenCounter) -> BatchResult<usize> {
    let input = TextInput::One(text.to_owned());
    let request = token_request(model, Some(&input), None);
    Ok(counter.count_tokens(&request).await?)
}

fn add_tokens(total: usize, contribution: usize) -> BatchResult<usize> {
    total
        .checked_add(contribution)
        .ok_or(BatchOutputUsageError::Overflow { field: "prompt" })
}
