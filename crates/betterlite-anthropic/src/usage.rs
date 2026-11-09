use std::error::Error as StdError;

use betterlite_models::decisions::Json;
use serde::Serialize;
use thiserror::Error;

mod cache_creation;
mod completion;
mod server_tool_use;
mod shape;
mod tokens;

use cache_creation::cache_creation_details;
use completion::{CompletionTokenInput, completion_token_details};
use server_tool_use::resolve_server_tool_use;
use shape::{UsageShapeError, optional_text, resolved_speed};
use tokens::{TokenCounts, iteration_blocks, token_counts};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AnthropicUsage {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub prompt_tokens_details: PromptTokensDetails,
    pub completion_tokens_details: CompletionTokensDetails,
    pub cache_creation_input_tokens: i64,
    pub cache_read_input_tokens: i64,
    pub iterations: Option<Json>,
    pub inference_geo: Option<String>,
    pub speed: Option<String>,
    pub service_tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_tool_use: Option<ServerToolUse>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PromptTokensDetails {
    #[serde(rename = "text_tokens")]
    pub text: i64,
    #[serde(rename = "cached_tokens")]
    pub cached: i64,
    #[serde(rename = "cache_creation_tokens")]
    pub cache_creation: i64,
    #[serde(rename = "cache_write_tokens")]
    pub cache_write: i64,
    #[serde(rename = "audio_tokens")]
    pub audio: Option<i64>,
    #[serde(rename = "image_tokens")]
    pub image: Option<i64>,
    #[serde(rename = "video_tokens")]
    pub video: Option<i64>,
    #[serde(
        rename = "cache_creation_token_details",
        skip_serializing_if = "Option::is_none"
    )]
    pub cache_creation_details: Option<CacheCreationTokenDetails>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CompletionTokensDetails {
    #[serde(rename = "reasoning_tokens")]
    pub reasoning: Option<i64>,
    #[serde(rename = "text_tokens")]
    pub text: Option<i64>,
    #[serde(rename = "accepted_prediction_tokens")]
    pub accepted_prediction: Option<i64>,
    #[serde(rename = "rejected_prediction_tokens")]
    pub rejected_prediction: Option<i64>,
    #[serde(rename = "audio_tokens")]
    pub audio: Option<i64>,
    #[serde(rename = "image_tokens")]
    pub image: Option<i64>,
    #[serde(rename = "video_tokens")]
    pub video: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CacheCreationTokenDetails {
    #[serde(rename = "ephemeral_5m_input_tokens")]
    pub ephemeral_5m: Option<i64>,
    #[serde(rename = "ephemeral_1h_input_tokens")]
    pub ephemeral_1h: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ServerToolUse {
    #[serde(rename = "web_search_requests")]
    pub web_search: Option<i64>,
    #[serde(rename = "tool_search_requests")]
    pub tool_search: Option<i64>,
    #[serde(rename = "browser_open_requests")]
    pub browser_open: Option<i64>,
}

#[derive(Debug, Error)]
pub enum UsageError<CounterFailure>
where
    CounterFailure: StdError + Send + Sync + 'static,
{
    #[error("the Anthropic usage object is not a mapping")]
    UsageNotMapping,
    #[error("the usage field {field} has a type the declared Anthropic shape does not allow")]
    FieldType { field: &'static str },
    #[error("the usage field {field} is outside the signed 64-bit range")]
    OutOfRange { field: &'static str },
    #[error("the usage arithmetic overflowed at {field}")]
    Overflow { field: &'static str },
    #[error("the reasoning token counter failed")]
    Counter {
        #[source]
        source: CounterFailure,
    },
}

impl<CounterFailure> From<UsageShapeError> for UsageError<CounterFailure>
where
    CounterFailure: StdError + Send + Sync + 'static,
{
    fn from(error: UsageShapeError) -> Self {
        match error {
            UsageShapeError::FieldType { field } => Self::FieldType { field },
            UsageShapeError::OutOfRange { field } => Self::OutOfRange { field },
            UsageShapeError::Overflow { field } => Self::Overflow { field },
        }
    }
}

// LiteLLM counts the fallback through token_counter(text=..., count_response_tokens=True), so the caller injects it.
pub trait ReasoningTokenCounter {
    type Failure: StdError + Send + Sync + 'static;

    fn count_reasoning_tokens(&self, text: &str) -> Result<usize, Self::Failure>;
}

pub fn calculate_usage<Counter>(
    usage_object: &Json,
    reasoning_content: Option<&str>,
    completion_response: Option<&Json>,
    speed: Option<&str>,
    counter: &Counter,
) -> Result<AnthropicUsage, UsageError<Counter::Failure>>
where
    Counter: ReasoningTokenCounter,
{
    let Json::Object(fields) = usage_object else {
        return Err(UsageError::UsageNotMapping);
    };
    let iterations = iteration_blocks(fields)?;
    let cache_creation_token_details = cache_creation_details(fields, iterations)?;
    let counts = token_counts(fields, iterations)?;
    let server_tool_use = resolve_server_tool_use(fields, completion_response)?;
    let completion_tokens_details = completion_token_details(
        CompletionTokenInput {
            fields,
            iterations,
            reasoning_content,
            completion_response,
            completion_tokens: counts.completion,
        },
        counter,
    )?;
    let prompt_tokens_details = prompt_token_details(counts, cache_creation_token_details)?;
    let total_tokens =
        counts
            .prompt
            .checked_add(counts.completion)
            .ok_or(UsageError::Overflow {
                field: "total_tokens",
            })?;
    Ok(AnthropicUsage {
        prompt_tokens: counts.prompt,
        completion_tokens: counts.completion,
        total_tokens,
        prompt_tokens_details,
        completion_tokens_details,
        cache_creation_input_tokens: counts.cache_creation,
        cache_read_input_tokens: counts.cache_read,
        iterations: fields.get("iterations").cloned(),
        inference_geo: optional_text(fields, "inference_geo")?,
        speed: resolved_speed(fields, speed),
        service_tier: optional_text(fields, "service_tier")?,
        server_tool_use,
    })
}

#[must_use]
pub fn is_anthropic_usage_object(usage_object: &Json) -> bool {
    let Json::Object(fields) = usage_object else {
        return false;
    };
    !fields.contains_key("prompt_tokens")
        && fields.contains_key("input_tokens")
        && (fields.contains_key("cache_read_input_tokens")
            || fields.contains_key("cache_creation_input_tokens"))
}

fn prompt_token_details(
    counts: TokenCounts,
    cache_creation_token_details: Option<CacheCreationTokenDetails>,
) -> Result<PromptTokensDetails, UsageShapeError> {
    Ok(PromptTokensDetails {
        text: counts
            .prompt
            .checked_sub(counts.cache_read)
            .and_then(|value| value.checked_sub(counts.cache_creation))
            .ok_or(UsageShapeError::Overflow {
                field: "prompt_tokens_details.text_tokens",
            })?,
        cached: counts.cache_read,
        cache_creation: counts.cache_creation,
        cache_write: counts.cache_creation,
        audio: None,
        image: None,
        video: None,
        cache_creation_details: cache_creation_token_details,
    })
}

mod tests;
