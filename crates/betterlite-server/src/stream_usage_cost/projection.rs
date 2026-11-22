use std::collections::BTreeMap;
use std::fmt::Display;
use std::str::FromStr as _;

use betterlite_anthropic::usage::{ReasoningTokenCounter, calculate_usage};
use betterlite_logging::fields::usage::Usage;
use betterlite_logging::fields::usage::parse::usage_from_mapping;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::cost_usage::CostUsage;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Number;

use super::StreamCostInjector;
use super::error::StreamCostError;
use super::pricing;

pub(super) fn inject_into_object(
    object: &Json,
    model_name: &str,
    injector: StreamCostInjector<'_>,
    log: Option<&mut LoggingState>,
) -> Result<Option<Json>, StreamCostError> {
    let Json::Object(fields) = object else {
        return Err(StreamCostError::NotAnObject);
    };
    let Some(Json::Object(usage)) = fields.get("usage") else {
        return Ok(None);
    };
    let Some(stream_usage) = event_usage(fields, usage)? else {
        return Ok(None);
    };
    let Some(cost) = pricing::streamed_cost(
        &stream_usage,
        model_name,
        service_tier(fields),
        injector,
        log,
    ) else {
        return Ok(None);
    };
    Ok(Some(with_cost(fields, usage, cost)))
}

// LiteLLM checks the Anthropic event type first, so an event that carries both tags stays Anthropic.
fn event_usage(
    fields: &IndexMap<String, Json>,
    usage: &IndexMap<String, Json>,
) -> Result<Option<Usage>, StreamCostError> {
    if matches!(
        fields.get("type"),
        Some(Json::Text(text)) if text == "message_delta"
    ) {
        return anthropic_usage(usage).map(Some);
    }
    if matches!(
        fields.get("object"),
        Some(Json::Text(text)) if text == "chat.completion.chunk"
    ) {
        return openai_usage(usage).map(Some);
    }
    Ok(None)
}

fn service_tier(fields: &IndexMap<String, Json>) -> Option<&str> {
    match fields.get("service_tier") {
        Some(Json::Text(tier)) => Some(tier.as_str()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

fn anthropic_usage(usage: &IndexMap<String, Json>) -> Result<Usage, StreamCostError> {
    let object = Json::Object(usage.clone());
    let calculated =
        calculate_usage(&object, None, None, None, &UnusedReasoningCounter).map_err(usage_error)?;
    let mapping = json_object(&calculated)?;
    usage_from_mapping(&mapping).map_err(usage_error)
}

fn openai_usage(counts: &IndexMap<String, Json>) -> Result<Usage, StreamCostError> {
    let prompt = integer(counts.get("prompt_tokens"), "prompt_tokens")?.unwrap_or(0);
    let completion = integer(counts.get("completion_tokens"), "completion_tokens")?.unwrap_or(0);
    let cost = CostUsage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: streamed_total(counts, prompt, completion)?,
        prompt_details: details(counts.get("prompt_tokens_details"), "prompt_tokens_details")?,
        completion_details: details(
            counts.get("completion_tokens_details"),
            "completion_tokens_details",
        )?,
        cache_read_tokens: cached_tokens(counts.get("prompt_tokens_details"))?,
        numeric_extras: BTreeMap::new(),
    };
    Ok(Usage::from(&cost))
}

fn streamed_total(
    counts: &IndexMap<String, Json>,
    prompt: i64,
    completion: i64,
) -> Result<i64, StreamCostError> {
    integer(counts.get("total_tokens"), "total_tokens")?.map_or_else(
        || {
            prompt
                .checked_add(completion)
                .ok_or(StreamCostError::Overflow {
                    field: "total_tokens",
                })
        },
        Ok,
    )
}

fn json_object<T>(value: &T) -> Result<IndexMap<String, Json>, StreamCostError>
where
    T: Serialize,
{
    let text = serde_json::to_string(value).map_err(|_error| StreamCostError::Serialize)?;
    let decoded =
        serde_json::from_str::<Json>(&text).map_err(|_error| StreamCostError::Serialize)?;
    let Json::Object(mapping) = decoded else {
        return Err(StreamCostError::Serialize);
    };
    Ok(mapping)
}

// Only a nonnull object carries details; any other declared value is the wrong type for the field.
fn details<T>(value: Option<&Json>, field: &'static str) -> Result<Option<T>, StreamCostError>
where
    T: DeserializeOwned,
{
    match value {
        Some(Json::Object(object)) => {
            let text =
                serde_json::to_string(object).map_err(|_error| StreamCostError::Serialize)?;
            let decoded = serde_json::from_str(&text).map_err(|_error| StreamCostError::Usage {
                reason: format!("the {field} field is not a token details object"),
            })?;
            Ok(Some(decoded))
        }
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(StreamCostError::DetailsNotAnObject { field })
        }
        None | Some(Json::Null) => Ok(None),
    }
}

// Usage::from reads cached_tokens back out of cache_read_tokens, so the value moves there first.
fn cached_tokens(value: Option<&Json>) -> Result<Option<i64>, StreamCostError> {
    let Some(Json::Object(found)) = value else {
        return Ok(None);
    };
    let Some(count) = found.get("cached_tokens") else {
        return Ok(None);
    };
    match count {
        Json::Null => Ok(None),
        Json::Number(number) => integer_number(number, "cached_tokens").map(Some),
        Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => {
            Err(StreamCostError::NotAnInteger {
                field: "cached_tokens",
            })
        }
    }
}

fn integer(value: Option<&Json>, field: &'static str) -> Result<Option<i64>, StreamCostError> {
    if let Some(Json::Number(number)) = value {
        return integer_number(number, field).map(Some);
    }
    if matches!(value, None | Some(Json::Null)) {
        return Ok(None);
    }
    Err(StreamCostError::NotAnInteger { field })
}

fn integer_number(number: &Number, field: &'static str) -> Result<i64, StreamCostError> {
    let value = decimal(number, field)?;
    if !value.fract().is_zero() {
        return Err(StreamCostError::NotAnInteger { field });
    }
    i64::try_from(value).map_err(|_error| StreamCostError::Overflow { field })
}

// A JSON float may arrive in exponent notation, which Decimal::from_str refuses; from_scientific reads it.
fn decimal(number: &Number, field: &'static str) -> Result<Decimal, StreamCostError> {
    let text = number.to_string();
    let parsed = if text.contains(['e', 'E']) {
        Decimal::from_scientific(&text)
    } else {
        Decimal::from_str(&text)
    };
    parsed.map_err(|_error| StreamCostError::NotANumber { field })
}

fn with_cost(
    fields: &IndexMap<String, Json>,
    usage: &IndexMap<String, Json>,
    cost: Decimal,
) -> Json {
    let mut updated_usage = usage.clone();
    updated_usage.insert("cost".to_owned(), pricing::cost_value(cost));
    let mut updated = fields.clone();
    updated.insert("usage".to_owned(), Json::Object(updated_usage));
    Json::Object(updated)
}

fn usage_error<E>(error: E) -> StreamCostError
where
    E: Display,
{
    StreamCostError::Usage {
        reason: error.to_string(),
    }
}

pub(super) struct UnusedReasoningCounter;

#[derive(Debug, thiserror::Error)]
#[error("the stream usage event carries no reasoning content to count")]
pub(super) struct NoReasoningContent;

impl ReasoningTokenCounter for UnusedReasoningCounter {
    type Failure = NoReasoningContent;

    fn count_reasoning_tokens(&self, _text: &str) -> Result<usize, Self::Failure> {
        Err(NoReasoningContent)
    }
}
