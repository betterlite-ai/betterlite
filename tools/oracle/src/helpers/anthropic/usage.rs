use betterlite_anthropic::usage::{
    AnthropicUsage, ReasoningTokenCounter, calculate_usage, is_anthropic_usage_object,
};
use betterlite_models::decisions::Json;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::helpers::{HelperCall, HelperError, invoke};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    usage: Json,
    #[serde(default)]
    reasoning_content: Option<String>,
    #[serde(default)]
    completion_response: Option<Json>,
    #[serde(default)]
    speed: Option<String>,
}

#[derive(Debug, thiserror::Error)]
#[error("the token_count helper failed: {message}")]
struct TokenCountFailure {
    message: String,
}

// The tokenizer helper owns the one Counter setup, so this counter drives its registered token_count operation.
struct TokenizerCounter;

impl ReasoningTokenCounter for TokenizerCounter {
    type Failure = TokenCountFailure;

    fn count_reasoning_tokens(&self, text: &str) -> Result<usize, Self::Failure> {
        let call = HelperCall {
            operation: "token_count".to_owned(),
            input: json!({
                "function": "token_counter",
                "model": "",
                "text": text,
                "count_response_tokens": true,
            }),
        };
        let output = invoke(&call);
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
        value
            .as_u64()
            .and_then(|count| usize::try_from(count).ok())
            .ok_or_else(|| TokenCountFailure {
                message: "the token_count helper returned a non-integer count".to_owned(),
            })
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let failure = |message: String| {
            Box::new(HelperError {
                class: "AnthropicUsage".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message,
            })
        };
        let usage = calculate_usage(
            &input.usage,
            input.reasoning_content.as_deref(),
            input.completion_response.as_ref(),
            input.speed.as_deref(),
            &TokenizerCounter,
        )
        .map_err(|error| failure(error.to_string()))?;
        serde_json::to_value(UsageAnswer {
            usage,
            is_anthropic_usage_object: is_anthropic_usage_object(&input.usage),
        })
        .map_err(|error| failure(error.to_string()))
    }
}

#[derive(Serialize)]
struct UsageAnswer {
    usage: AnthropicUsage,
    is_anthropic_usage_object: bool,
}

mod tests;
