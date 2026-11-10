use betterlite_bedrock::converse_usage::cache_details::parse_cache_details;
use betterlite_bedrock::converse_usage::reasoning::{
    thinking_tokens_from_fields, transform_reasoning_content, transform_thinking_blocks,
};
use betterlite_bedrock::converse_usage::transform::{
    ReasoningTokenCounter, transform_usage, usage_from_batch_output,
};
use betterlite_bedrock::converse_usage::wire::{ConverseTokenUsage, is_converse_usage_shape};
use betterlite_models::decisions::Json;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::helpers::{HelperCall, HelperError, HelperResult, invoke};

mod tests;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Operation {
    TransformUsage {
        usage: Json,
        #[serde(default)]
        reasoning_content: Option<String>,
        #[serde(default)]
        thinking_ran: bool,
        #[serde(default)]
        provider_reasoning_tokens: Option<i64>,
    },
    Batch {
        usage: Json,
    },
    Shape {
        usage: Json,
    },
    CacheDetails {
        usage: Json,
    },
    AdditionalThinkingTokens {
        #[serde(default)]
        additional_fields: Option<Json>,
        #[serde(default)]
        usage: Option<Json>,
        #[serde(default)]
        reasoning_content: Option<String>,
        #[serde(default)]
        thinking_ran: bool,
        #[serde(default)]
        provider_reasoning_tokens: Option<i64>,
    },
    ReasoningBlocks {
        blocks: Vec<Json>,
    },
    TotalTokens {
        usage: Json,
    },
}

fn failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ConverseUsage".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn payload<Payload>(value: Payload) -> HelperResult
where
    Payload: Serialize,
{
    match serde_json::to_value(value) {
        Ok(json) => Ok(json),
        Err(error) => Err(failure(error.to_string())),
    }
}

fn decode_usage(value: &Json) -> Result<ConverseTokenUsage, Box<HelperError>> {
    match ConverseTokenUsage::decode(value) {
        Ok(usage) => Ok(usage),
        Err(error) => Err(failure(error.to_string())),
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the token_count helper failed: {message}")]
struct TokenCountFailure {
    message: String,
}

#[derive(Serialize)]
struct TokenCountRequest<'text> {
    function: &'static str,
    model: &'static str,
    text: &'text str,
    count_response_tokens: bool,
}

struct TokenizerCounter;

impl ReasoningTokenCounter for TokenizerCounter {
    type Failure = TokenCountFailure;

    fn count_reasoning_tokens(&self, text: &str) -> Result<usize, Self::Failure> {
        let request = TokenCountRequest {
            function: "token_counter",
            model: "",
            text,
            count_response_tokens: true,
        };
        let input = serde_json::to_value(request).map_err(|error| TokenCountFailure {
            message: error.to_string(),
        })?;
        let output = invoke(&HelperCall {
            operation: "token_count".to_owned(),
            input,
        });
        if let Some(error) = output.error {
            return Err(TokenCountFailure {
                message: error.message,
            });
        }
        let Some(value) = output.value else {
            return Err(TokenCountFailure {
                message: "the token_count helper returned no count".to_owned(),
            });
        };
        serde_json::from_value(value).map_err(|error| TokenCountFailure {
            message: error.to_string(),
        })
    }
}

fn transform(
    usage: &Json,
    reasoning_content: Option<&str>,
    thinking_ran: bool,
    provider_reasoning_tokens: Option<i64>,
) -> HelperResult {
    let block = decode_usage(usage)?;
    let projected = transform_usage(
        &block,
        reasoning_content,
        thinking_ran,
        provider_reasoning_tokens,
        &TokenizerCounter,
    )
    .map_err(|error| failure(error.to_string()))?;
    payload(projected)
}

fn additional(
    additional_fields: Option<&Json>,
    usage: Option<&Json>,
    reasoning_content: Option<&str>,
    thinking_ran: bool,
    provider_reasoning_tokens: Option<i64>,
) -> HelperResult {
    let tokens = additional_fields.and_then(thinking_tokens_from_fields);
    let Some(supplied) = usage else {
        return payload(json!({ "thinking_tokens": tokens }));
    };
    let block = decode_usage(supplied)?;
    let projected = transform_usage(
        &block,
        reasoning_content,
        thinking_ran,
        provider_reasoning_tokens,
        &TokenizerCounter,
    )
    .map_err(|error| failure(error.to_string()))?;
    payload(json!({
        "thinking_tokens": tokens,
        "usage": serde_json::to_value(&projected).map_err(|error| failure(error.to_string()))?,
    }))
}

fn batch(usage: &Json) -> HelperResult {
    let block = decode_usage(usage)?;
    let projected = usage_from_batch_output(&block).map_err(|error| failure(error.to_string()))?;
    payload(json!({
        "usage": projected,
        "shape": is_converse_usage_shape(usage),
    }))
}

fn cache_details(usage: &Json) -> HelperResult {
    let block = decode_usage(usage)?;
    let details = parse_cache_details(&block).map_err(|error| failure(error.to_string()))?;
    payload(details)
}

fn reasoning_blocks(blocks: &[Json]) -> HelperResult {
    let content =
        transform_reasoning_content(blocks).map_err(|error| failure(error.to_string()))?;
    let thinking = transform_thinking_blocks(blocks).map_err(|error| failure(error.to_string()))?;
    payload(json!({
        "reasoning_content": content,
        "thinking_blocks": thinking,
    }))
}

fn totals(usage: &Json) -> HelperResult {
    let block = decode_usage(usage)?;
    let live = transform_usage(&block, None, false, None, &TokenizerCounter)
        .map_err(|error| failure(error.to_string()))?;
    let batch = usage_from_batch_output(&block).map_err(|error| failure(error.to_string()))?;
    payload(json!({
        "live": serde_json::to_value(&live).map_err(|error| failure(error.to_string()))?,
        "batch": serde_json::to_value(&batch).map_err(|error| failure(error.to_string()))?,
    }))
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Operation;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Operation::TransformUsage {
                usage,
                reasoning_content,
                thinking_ran,
                provider_reasoning_tokens,
            } => transform(
                usage,
                reasoning_content.as_deref(),
                *thinking_ran,
                *provider_reasoning_tokens,
            ),
            Operation::Batch { usage } => batch(usage),
            Operation::Shape { usage } => payload(is_converse_usage_shape(usage)),
            Operation::CacheDetails { usage } => cache_details(usage),
            Operation::AdditionalThinkingTokens {
                additional_fields,
                usage,
                reasoning_content,
                thinking_ran,
                provider_reasoning_tokens,
            } => additional(
                additional_fields.as_ref(),
                usage.as_ref(),
                reasoning_content.as_deref(),
                *thinking_ran,
                *provider_reasoning_tokens,
            ),
            Operation::ReasoningBlocks { blocks } => reasoning_blocks(blocks),
            Operation::TotalTokens { usage } => totals(usage),
        }
    }
}
