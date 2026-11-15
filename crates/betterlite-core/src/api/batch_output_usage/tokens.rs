use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_tokenizer::count::Counter;
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::{Message, TextInput, TokenRequest};

use super::decode::number_is_integer;
use super::error::BatchEntryTokenError;

// A batch request's input tokens scale roughly with its serialized size, so this is a conservative per-row fallback.
pub const TOKEN_ESTIMATE_BYTES_PER_TOKEN: usize = 4;

#[must_use]
pub fn estimate_batch_entry_tokens(raw_line: &[u8]) -> usize {
    let estimate = raw_line.len().div_euclid(TOKEN_ESTIMATE_BYTES_PER_TOKEN);
    estimate.max(1)
}

// B013's local token counter is async; the batch input path injects it instead of reaching for a runtime.
#[async_trait]
pub trait BatchTokenCounter: Send + Sync {
    async fn count_tokens(&self, request: &TokenRequest<'_>) -> Result<usize, CountError>;
}

#[async_trait]
impl BatchTokenCounter for Counter {
    async fn count_tokens(&self, request: &TokenRequest<'_>) -> Result<usize, CountError> {
        self.count(request).await
    }
}

pub async fn count_batch_entry_tokens(
    entry: &Json,
    model_name: Option<&str>,
    counter: &dyn BatchTokenCounter,
) -> Result<usize, BatchEntryTokenError> {
    let Json::Object(fields) = entry else {
        return Err(BatchEntryTokenError::NotAnEntry);
    };
    let body = match fields.get("body") {
        None | Some(Json::Null) => return Ok(0),
        Some(Json::Object(body)) => body,
        Some(_) => return Err(BatchEntryTokenError::NotAnObject { field: "body" }),
    };
    let model = entry_model(body.get("model"), model_name)?;
    if let Some(messages) = present(body.get("messages")) {
        return count_messages(model, messages, counter).await;
    }
    if let Some(prompt) = present(body.get("prompt")) {
        return count_prompt_value(model, prompt, counter).await;
    }
    if let Some(input) = present(body.get("input")) {
        return count_prompt_value(model, input, counter).await;
    }
    Ok(0)
}

fn entry_model<'model>(
    declared: Option<&'model Json>,
    model_name: Option<&'model str>,
) -> Result<&'model str, BatchEntryTokenError> {
    match declared {
        None | Some(Json::Null) => Ok(model_name.unwrap_or_default()),
        Some(Json::Text(model)) => Ok(model.as_str()),
        Some(_) => Err(BatchEntryTokenError::NotAString { field: "model" }),
    }
}

fn present(value: Option<&Json>) -> Option<&Json> {
    value.filter(|found| !matches!(found, Json::Null))
}

async fn count_messages(
    model: &str,
    messages: &Json,
    counter: &dyn BatchTokenCounter,
) -> Result<usize, BatchEntryTokenError> {
    let value = serde_json::to_value(messages)
        .map_err(|source| BatchEntryTokenError::Messages { source })?;
    let decoded: Vec<Message> = serde_json::from_value(value)
        .map_err(|source| BatchEntryTokenError::Messages { source })?;
    let request = TokenRequest {
        model,
        custom_tokenizer: None,
        text: None,
        messages: Some(&decoded),
        count_response_tokens: Some(false),
        tools: None,
        tool_choice: None,
    };
    Ok(counter.count_tokens(&request).await?)
}

async fn count_prompt_value(
    model: &str,
    value: &Json,
    counter: &dyn BatchTokenCounter,
) -> Result<usize, BatchEntryTokenError> {
    match value {
        Json::Text(text) => count_text(model, text, counter).await,
        Json::List(items) => {
            let mut total = 0_usize;
            for item in items {
                let contribution = count_prompt_chunk(model, item, counter).await?;
                total = add_tokens(total, contribution, "prompt")?;
            }
            Ok(total)
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_) => Ok(0),
    }
}

async fn count_prompt_chunk(
    model: &str,
    chunk: &Json,
    counter: &dyn BatchTokenCounter,
) -> Result<usize, BatchEntryTokenError> {
    match chunk {
        Json::Text(text) => count_text(model, text, counter).await,
        Json::Number(number) if number_is_integer(number) => Ok(1),
        Json::Bool(_) => Ok(1),
        Json::List(nested) => {
            let mut total = 0_usize;
            for item in nested {
                let contribution = match item {
                    Json::Text(text) => count_text(model, text, counter).await?,
                    Json::Number(number) if number_is_integer(number) => 1,
                    Json::Bool(_) => 1,
                    Json::Null | Json::Number(_) | Json::List(_) | Json::Object(_) => 0,
                };
                total = add_tokens(total, contribution, "prompt")?;
            }
            Ok(total)
        }
        Json::Null | Json::Number(_) | Json::Object(_) => Ok(0),
    }
}

async fn count_text(
    model: &str,
    text: &str,
    counter: &dyn BatchTokenCounter,
) -> Result<usize, BatchEntryTokenError> {
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
    Ok(counter.count_tokens(&request).await?)
}

fn add_tokens(
    total: usize,
    contribution: usize,
    field: &'static str,
) -> Result<usize, BatchEntryTokenError> {
    total
        .checked_add(contribution)
        .ok_or(BatchEntryTokenError::Overflow { field })
}
