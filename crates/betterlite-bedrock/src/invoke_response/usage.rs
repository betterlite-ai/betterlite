use std::time::{Duration, SystemTime};

use betterlite_openai::chat::response::{ChatResponse, ChatUsage};
use betterlite_tokenizer::count::error::CountError;
use num_traits::cast;

use crate::image_models::sizing::parse_python_int;
use crate::invoke_response::error::InvokeReplyError;
use crate::invoke_response::{InvokeReplyCall, RawInvokeReply};
use crate::pegasus::counter::TokenCounter;

const INPUT_TOKENS: &str = "x-amzn-bedrock-input-token-count";
const OUTPUT_TOKENS: &str = "x-amzn-bedrock-output-token-count";

pub(super) async fn assign<Counter>(
    call: &InvokeReplyCall<'_, '_, '_>,
    raw: &RawInvokeReply<'_>,
    response: &mut ChatResponse,
    counter: &Counter,
) -> Result<(), InvokeReplyError>
where
    Counter: TokenCounter + Sync,
{
    let prompt = match header_count(raw.headers, INPUT_TOKENS)? {
        Some(count) => count,
        None => counter
            .count_messages(call.counted_messages)
            .await
            .map_err(|source| InvokeReplyError::TokenCount { source })?,
    };
    let completion = match header_count(raw.headers, OUTPUT_TOKENS)? {
        Some(count) => count,
        None => completion_count(response, counter).await?,
    };
    let total = i128::from(prompt) + i128::from(completion);
    response.usage = ChatUsage {
        prompt_tokens: in_range(i128::from(prompt), "prompt_tokens")?,
        completion_tokens: in_range(i128::from(completion), "completion_tokens")?,
        total_tokens: in_range(total, "total_tokens")?,
        prompt_tokens_details: None,
        completion_tokens_details: None,
    };
    response.created = created(call.now)?;
    call.model.clone_into(&mut response.model);
    Ok(())
}

// LiteLLM passes the full model_response through, so a seeded content is what gets counted.
async fn completion_count<Counter>(
    response: &ChatResponse,
    counter: &Counter,
) -> Result<i64, InvokeReplyError>
where
    Counter: TokenCounter + Sync,
{
    let text = response
        .choices
        .first()
        .and_then(|choice| choice.message.content.as_deref())
        .ok_or(InvokeReplyError::TokenCount {
            source: CountError::MissingInput,
        })?;
    counter
        .count_text(text)
        .await
        .map_err(|source| InvokeReplyError::TokenCount { source })
}

fn header_count(
    headers: &[(String, String)],
    name: &'static str,
) -> Result<Option<i64>, InvokeReplyError> {
    let Some(value) = header(headers, name).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    parse_python_int(value)
        .map(Some)
        .map_err(|source| InvokeReplyError::TokenCountHeader { name, source })
}

fn header<'headers>(headers: &'headers [(String, String)], name: &str) -> Option<&'headers str> {
    headers
        .iter()
        .find(|(header_name, _value)| header_name.eq_ignore_ascii_case(name))
        .map(|(_name, value)| value.as_str())
}

fn in_range(value: i128, name: &'static str) -> Result<u64, InvokeReplyError> {
    u64::try_from(value).map_err(|_error| InvokeReplyError::UsageRange { name })
}

fn created(now: f64) -> Result<u64, InvokeReplyError> {
    let seconds = seconds(now)?;
    u64::try_from(seconds).map_err(|_error| InvokeReplyError::CreatedOutOfRange)
}

pub(super) fn seconds(now: f64) -> Result<i64, InvokeReplyError> {
    cast::<f64, i64>(now).ok_or(InvokeReplyError::ClockOutOfRange)
}

pub(super) fn instant(now: f64) -> Result<SystemTime, InvokeReplyError> {
    let magnitude = Duration::try_from_secs_f64(now.abs())
        .map_err(|_error| InvokeReplyError::ClockOutOfRange)?;
    let instant = if now.is_sign_negative() {
        SystemTime::UNIX_EPOCH.checked_sub(magnitude)
    } else {
        SystemTime::UNIX_EPOCH.checked_add(magnitude)
    };
    instant.ok_or(InvokeReplyError::ClockOutOfRange)
}
