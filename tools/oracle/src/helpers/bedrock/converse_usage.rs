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

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct ReasoningFields {
    reasoning_content: Option<String>,
    thinking_ran: bool,
    provider_reasoning_tokens: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Operation {
    TransformUsage {
        usage: Json,
        #[serde(default)]
        additional_fields: Option<Json>,
        #[serde(flatten)]
        reasoning: ReasoningFields,
    },
    Batch {
        usage: Json,
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

    fn count_reasoning_tokens(&self, text: &str) -> Result<i64, Self::Failure> {
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
    reasoning: &ReasoningFields,
    additional_fields: Option<&Json>,
) -> HelperResult {
    let block = decode_usage(usage)?;
    let projected = transform_usage(
        &block,
        reasoning.reasoning_content.as_deref(),
        reasoning.thinking_ran,
        reasoning.provider_reasoning_tokens,
        &TokenizerCounter,
    )
    .map_err(|error| failure(error.to_string()))?;
    let tokens = additional_fields.and_then(thinking_tokens_from_fields);
    payload(json!({
        "usage": projected,
        "thinking_tokens": tokens,
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
    payload((live, batch))
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Operation;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Operation::TransformUsage {
                usage,
                additional_fields,
                reasoning,
            } => transform(usage, reasoning, additional_fields.as_ref()),
            Operation::Batch { usage } => batch(usage),
            Operation::ReasoningBlocks { blocks } => reasoning_blocks(blocks),
            Operation::TotalTokens { usage } => totals(usage),
        }
    }
}
