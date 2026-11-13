use std::error::Error as StdError;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use crate::converse_usage::cache_details::{CacheCreationTokenDetails, parse_cache_details};
use crate::converse_usage::counts::{cache_counts, count_value, live_count, number_value};
use crate::converse_usage::error::{ConverseUsageError, UsageError};
use crate::converse_usage::wire::ConverseTokenUsage;
use crate::converse_usage::{CompletionTokensDetails, PromptTokensDetails, Usage};

pub trait ReasoningTokenCounter {
    type Failure: StdError + Send + Sync + 'static;

    fn count_reasoning_tokens(&self, text: &str) -> Result<i64, Self::Failure>;
}

pub fn transform_usage<Counter>(
    usage: &ConverseTokenUsage,
    reasoning_content: Option<&str>,
    thinking_ran: bool,
    provider_reasoning_tokens: Option<i64>,
    counter: &Counter,
) -> Result<Usage, UsageError<Counter::Failure>>
where
    Counter: ReasoningTokenCounter,
{
    let projected = project(usage)?;
    let estimate = match reasoning_content.filter(|content| !content.is_empty()) {
        Some(content) => counter
            .count_reasoning_tokens(content)
            .map_err(|source| UsageError::Counter { source })?,
        None => 0,
    };
    Ok(assemble(
        projected,
        estimate,
        thinking_ran,
        provider_reasoning_tokens,
    )?)
}

// Batch output omits fields the live API always sends, so the counts are completed before the same projection runs, and cacheDetails stays behind.
pub fn usage_from_batch_output(usage: &ConverseTokenUsage) -> Result<Usage, ConverseUsageError> {
    let input = count_value(usage.input_tokens.as_ref(), "inputTokens")?.unwrap_or(0);
    let output = count_value(usage.output_tokens.as_ref(), "outputTokens")?.unwrap_or(0);
    let (cache_read, cache_creation) = cache_counts(usage)?;
    let supplied = count_value(usage.remaining.get("totalTokens"), "totalTokens")?.unwrap_or(0);
    let total = if supplied == 0 {
        input
            .checked_add(output)
            .ok_or(ConverseUsageError::Overflow {
                field: "totalTokens",
            })?
    } else {
        supplied
    };
    let completed = ConverseTokenUsage {
        input_tokens: Some(Json::Number(Number::from(input))),
        output_tokens: Some(Json::Number(Number::from(output))),
        cache_read_input_tokens: Some(Json::Number(Number::from(cache_read))),
        cache_read_input_token_count: Some(Json::Number(Number::from(cache_read))),
        cache_write_input_tokens: Some(Json::Number(Number::from(cache_creation))),
        cache_write_input_token_count: Some(Json::Number(Number::from(cache_creation))),
        cache_details: None,
        remaining: IndexMap::from([("totalTokens".to_owned(), Json::Number(Number::from(total)))]),
    };
    assemble(project(&completed)?, 0, false, None)
}

struct ProjectedUsage {
    raw_input: i64,
    output: i64,
    cache_read: i64,
    cache_creation: i64,
    input: i64,
    total: i64,
    details: Option<CacheCreationTokenDetails>,
}

fn project(usage: &ConverseTokenUsage) -> Result<ProjectedUsage, ConverseUsageError> {
    let raw_input = live_count(usage.input_tokens.as_ref(), "inputTokens")?;
    let output = live_count(usage.output_tokens.as_ref(), "outputTokens")?;
    let (cache_read, cache_creation) = cache_counts(usage)?;
    let input = raw_input
        .checked_add(cache_read)
        .and_then(|sum| sum.checked_add(cache_creation))
        .ok_or(ConverseUsageError::Overflow {
            field: "prompt_tokens",
        })?;
    let total = match usage.remaining.get("totalTokens") {
        None => input
            .checked_add(output)
            .ok_or(ConverseUsageError::Overflow {
                field: "total_tokens",
            })?,
        Some(Json::Null) => 0,
        Some(Json::Number(number)) => number_value(number, "totalTokens", false)?,
        Some(_) => {
            return Err(ConverseUsageError::FieldType {
                field: "totalTokens",
            });
        }
    };
    let details = parse_cache_details(usage)?;
    Ok(ProjectedUsage {
        raw_input,
        output,
        cache_read,
        cache_creation,
        input,
        total,
        details,
    })
}

fn assemble(
    projected: ProjectedUsage,
    estimate: i64,
    thinking_ran: bool,
    provider_reasoning_tokens: Option<i64>,
) -> Result<Usage, ConverseUsageError> {
    let reasoning =
        provider_reasoning_tokens.map_or(estimate, |count| count.max(0).min(projected.output));
    let (reasoning_tokens, text_tokens) =
        completion_split(projected.output, reasoning, thinking_ran)?;
    Ok(Usage {
        completion_tokens: projected.output,
        prompt_tokens: projected.input,
        total_tokens: projected.total,
        completion_tokens_details: CompletionTokensDetails {
            accepted_prediction_tokens: None,
            audio_tokens: None,
            reasoning_tokens,
            rejected_prediction_tokens: None,
            text_tokens,
            image_tokens: None,
            video_tokens: None,
        },
        prompt_tokens_details: PromptTokensDetails {
            audio_tokens: None,
            cache_write_tokens: projected.cache_creation,
            cached_tokens: projected.cache_read,
            text_tokens: projected.raw_input,
            image_tokens: None,
            video_tokens: None,
            cache_creation_tokens: projected.cache_creation,
            cache_creation_token_details: projected.details,
        },
        cache_creation_input_tokens: projected.cache_creation,
        cache_read_input_tokens: projected.cache_read,
    })
}

fn completion_split(
    output: i64,
    reasoning: i64,
    thinking_ran: bool,
) -> Result<(Option<i64>, Option<i64>), ConverseUsageError> {
    if reasoning > 0 {
        let text = output
            .checked_sub(reasoning)
            .ok_or(ConverseUsageError::Overflow {
                field: "completion_tokens_details.text_tokens",
            })?;
        return Ok((Some(reasoning), Some(text)));
    }
    if thinking_ran {
        return Ok((None, None));
    }
    Ok((Some(0), Some(output)))
}
