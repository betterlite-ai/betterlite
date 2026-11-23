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
    let total = i128::from(prompt)
        .checked_add(i128::from(completion))
        .ok_or(InvokeReplyError::UsageRange {
            name: "total_tokens",
        })?;
    response.usage = ChatUsage {
        prompt_tokens: u64::try_from(i128::from(prompt)).map_err(|_error| {
            InvokeReplyError::UsageRange {
                name: "prompt_tokens",
            }
        })?,
        completion_tokens: u64::try_from(i128::from(completion)).map_err(|_error| {
            InvokeReplyError::UsageRange {
                name: "completion_tokens",
            }
        })?,
        total_tokens: u64::try_from(total).map_err(|_error| InvokeReplyError::UsageRange {
            name: "total_tokens",
        })?,
        prompt_tokens_details: None,
        completion_tokens_details: None,
    };
    response.created =
        u64::try_from(cast::<f64, i64>(call.now).ok_or(InvokeReplyError::ClockOutOfRange)?)
            .map_err(|_error| InvokeReplyError::CreatedOutOfRange)?;
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
    let Some(value) = headers
        .iter()
        .find(|(header_name, _value)| header_name.eq_ignore_ascii_case(name))
        .map(|(_name, value)| value.as_str())
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    parse_python_int(value)
        .map(Some)
        .map_err(|source| InvokeReplyError::TokenCountHeader { name, source })
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
