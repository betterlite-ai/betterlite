use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::shape::{UsageShapeError, integer_value};
use super::{CompletionTokensDetails, ReasoningTokenCounter, UsageError};
use crate::response_content::wire::{get, json_text};

type CompletionSplit = (Option<i64>, Option<i64>);

#[derive(Clone, Copy)]
pub(super) struct CompletionTokenInput<'usage> {
    pub(super) fields: &'usage IndexMap<String, Json>,
    pub(super) iterations: Option<&'usage [Json]>,
    pub(super) reasoning_content: Option<&'usage str>,
    pub(super) completion_response: Option<&'usage Json>,
    pub(super) completion_tokens: i64,
}

pub(super) fn completion_token_details<Counter>(
    input: CompletionTokenInput<'_>,
    counter: &Counter,
) -> Result<CompletionTokensDetails, UsageError<Counter::Failure>>
where
    Counter: ReasoningTokenCounter,
{
    let (reasoning, text) = completion_token_split(input, counter)?;
    Ok(CompletionTokensDetails {
        reasoning,
        text,
        accepted_prediction: None,
        rejected_prediction: None,
        audio: None,
        image: None,
        video: None,
    })
}

fn completion_token_split<Counter>(
    input: CompletionTokenInput<'_>,
    counter: &Counter,
) -> Result<CompletionSplit, UsageError<Counter::Failure>>
where
    Counter: ReasoningTokenCounter,
{
    let reported = iteration_thinking_tokens(input.iterations)?
        .or_else(|| thinking_tokens_from_usage(input.fields));
    if let Some(tokens) = reported {
        let reasoning = tokens.max(0).min(input.completion_tokens);
        return Ok(completion_text_split(input.completion_tokens, reasoning)?);
    }
    if let Some(reasoning_content) = input.reasoning_content.filter(|text| !text.is_empty()) {
        let token_count = counter
            .count_reasoning_tokens(reasoning_content)
            .map_err(|source| UsageError::Counter { source })?;
        let estimate = i64::try_from(token_count)
            .ok()
            .ok_or(UsageShapeError::OutOfRange {
                field: "reasoning_tokens",
            })?;
        return Ok(completion_text_split(
            input.completion_tokens,
            estimate.min(input.completion_tokens).max(0),
        )?);
    }
    if response_has_thinking_block(input.completion_response) {
        return Ok((None, None));
    }
    Ok((Some(0), Some(input.completion_tokens)))
}

fn completion_text_split(
    completion_tokens: i64,
    reasoning_tokens: i64,
) -> Result<CompletionSplit, UsageShapeError> {
    let text_tokens =
        completion_tokens
            .checked_sub(reasoning_tokens)
            .ok_or(UsageShapeError::Overflow {
                field: "completion_tokens_details.text_tokens",
            })?;
    Ok((Some(reasoning_tokens), Some(text_tokens)))
}

fn iteration_thinking_tokens(iterations: Option<&[Json]>) -> Result<Option<i64>, UsageShapeError> {
    let Some(items) = iterations else {
        return Ok(None);
    };
    let mut total = 0_i64;
    for item in items {
        let Json::Object(fields) = item else {
            return Ok(None);
        };
        let Some(tokens) = thinking_tokens_from_usage(fields) else {
            return Ok(None);
        };
        total = total.checked_add(tokens).ok_or(UsageShapeError::Overflow {
            field: "completion_tokens_details.reasoning_tokens",
        })?;
    }
    Ok(Some(total))
}

fn thinking_tokens_from_usage(fields: &IndexMap<String, Json>) -> Option<i64> {
    let Some(Json::Object(details)) = fields.get("output_tokens_details") else {
        return None;
    };
    match details.get("thinking_tokens") {
        Some(Json::Number(number)) => {
            integer_value(number, "output_tokens_details.thinking_tokens").ok()
        }
        Some(Json::Bool(flag)) => Some(i64::from(*flag)),
        None | Some(Json::Null | Json::Text(_) | Json::List(_) | Json::Object(_)) => None,
    }
}

fn response_has_thinking_block(completion_response: Option<&Json>) -> bool {
    let Some(Json::Object(response)) = completion_response else {
        return false;
    };
    let Some(Json::List(content)) = response.get("content") else {
        return false;
    };
    content.iter().any(|block| {
        matches!(
            get(block, "type").and_then(json_text),
            Some("thinking" | "redacted_thinking")
        )
    })
}
