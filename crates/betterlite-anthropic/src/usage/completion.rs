use std::error::Error as StdError;

use betterlite_models::decisions::Json;

use super::count::thinking_tokens;
use super::error::UsageError;
use super::wire::{OutputTokensDetailsValue, UsageIteration, UsageObject};
use super::{CompletionTokensDetails, ReasoningTokenCounter};
use crate::response_content::wire::{get, json_text};

type CompletionSplit = (Option<i64>, Option<i64>);

#[derive(Clone, Copy)]
pub(super) struct CompletionTokenInput<'usage> {
    pub(super) object: &'usage UsageObject,
    pub(super) iterations: Option<&'usage [UsageIteration]>,
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
        .or_else(|| details_thinking(input.object.output_tokens_details.as_ref()));
    if let Some(tokens) = reported {
        let reasoning = tokens.max(0).min(input.completion_tokens);
        return completion_text_split(input.completion_tokens, reasoning);
    }
    if let Some(reasoning_content) = input.reasoning_content.filter(|text| !text.is_empty()) {
        let token_count = counter
            .count_reasoning_tokens(reasoning_content)
            .map_err(|source| UsageError::Counter { source })?;
        let estimate = i64::try_from(token_count)
            .ok()
            .ok_or(UsageError::OutOfRange {
                field: "reasoning_tokens",
            })?;
        return completion_text_split(
            input.completion_tokens,
            estimate.min(input.completion_tokens).max(0),
        );
    }
    if response_has_thinking_block(input.completion_response) {
        return Ok((None, None));
    }
    Ok((Some(0), Some(input.completion_tokens)))
}

fn completion_text_split<CounterFailure>(
    completion_tokens: i64,
    reasoning_tokens: i64,
) -> Result<CompletionSplit, UsageError<CounterFailure>>
where
    CounterFailure: StdError + Send + Sync + 'static,
{
    let text_tokens =
        completion_tokens
            .checked_sub(reasoning_tokens)
            .ok_or(UsageError::Overflow {
                field: "completion_tokens_details.text_tokens",
            })?;
    Ok((Some(reasoning_tokens), Some(text_tokens)))
}

fn iteration_thinking_tokens<CounterFailure>(
    iterations: Option<&[UsageIteration]>,
) -> Result<Option<i64>, UsageError<CounterFailure>>
where
    CounterFailure: StdError + Send + Sync + 'static,
{
    let Some(items) = iterations else {
        return Ok(None);
    };
    let mut total = 0_i64;
    for iteration in items {
        let Some(tokens) = details_thinking(iteration.output_tokens_details.as_ref()) else {
            return Ok(None);
        };
        total = total.checked_add(tokens).ok_or(UsageError::Overflow {
            field: "completion_tokens_details.reasoning_tokens",
        })?;
    }
    Ok(Some(total))
}

fn details_thinking(value: Option<&OutputTokensDetailsValue>) -> Option<i64> {
    match value? {
        OutputTokensDetailsValue::Mapping(found) => thinking_tokens(found.thinking_tokens.as_ref()),
        OutputTokensDetailsValue::Other => None,
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
