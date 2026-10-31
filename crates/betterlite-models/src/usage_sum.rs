use std::collections::BTreeMap;

use rust_decimal::Decimal;
use thiserror::Error;

use crate::cost_usage::{CompletionDetails, CostUsage, PromptDetails};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UsageNumber {
    Integer(i64),
    Decimal(Decimal),
    Boolean(bool),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CachedTokensDetails {
    pub text_tokens: Option<i64>,
    pub audio_tokens: Option<i64>,
    pub image_tokens: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PromptTokensDetails {
    pub text_tokens: Option<i64>,
    pub audio_tokens: Option<i64>,
    pub image_tokens: Option<i64>,
    pub video_tokens: Option<i64>,
    pub web_search_requests: Option<i64>,
    pub google_maps_grounding_requests: Option<i64>,
    pub tool_use_tokens: Option<i64>,
    pub character_count: Option<i64>,
    pub image_count: Option<i64>,
    pub video_length_seconds: Option<Decimal>,
    pub audio_length_seconds: Option<Decimal>,
    pub query_count: Option<i64>,
    pub cache_write_tokens: Option<i64>,
    pub cache_creation_tokens: Option<i64>,
    pub cached_tokens_details: Option<CachedTokensDetails>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CompletionTokensDetails {
    pub text_tokens: Option<i64>,
    pub audio_tokens: Option<i64>,
    pub image_tokens: Option<i64>,
    pub video_tokens: Option<i64>,
    pub reasoning_tokens: Option<i64>,
    pub accepted_prediction_tokens: Option<i64>,
    pub rejected_prediction_tokens: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Usage {
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub total_tokens: Option<i64>,
    pub prompt_tokens_details: Option<PromptTokensDetails>,
    pub completion_tokens_details: Option<CompletionTokensDetails>,
    pub numeric_extras: BTreeMap<String, UsageNumber>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum UsageSumError {
    #[error("usage counter overflow for {field}")]
    IntegerOverflow { field: String },
    #[error("usage decimal overflow for {field}")]
    DecimalOverflow { field: String },
}

#[must_use]
pub fn combine_cached_tokens_details(
    current: Option<&CachedTokensDetails>,
    new: &CachedTokensDetails,
) -> CachedTokensDetails {
    let result = CachedTokensDetails {
        text_tokens: sum_optional(current.and_then(|value| value.text_tokens), new.text_tokens),
        audio_tokens: sum_optional(
            current.and_then(|value| value.audio_tokens),
            new.audio_tokens,
        ),
        image_tokens: sum_optional(
            current.and_then(|value| value.image_tokens),
            new.image_tokens,
        ),
    };
    result
}

#[must_use]
pub fn combine_prompt_tokens_details(
    current: Option<&PromptTokensDetails>,
    new: &PromptTokensDetails,
) -> PromptTokensDetails {
    let base = current.cloned().unwrap_or_default();
    let cache_write_tokens = new
        .cache_write_tokens
        .map_or(base.cache_write_tokens, |value| {
            sum_optional(base.cache_write_tokens, Some(value))
        });
    PromptTokensDetails {
        text_tokens: sum_optional(base.text_tokens, new.text_tokens),
        audio_tokens: sum_optional(base.audio_tokens, new.audio_tokens),
        image_tokens: sum_optional(base.image_tokens, new.image_tokens),
        video_tokens: sum_optional(base.video_tokens, new.video_tokens),
        web_search_requests: sum_optional(base.web_search_requests, new.web_search_requests),
        google_maps_grounding_requests: sum_optional(
            base.google_maps_grounding_requests,
            new.google_maps_grounding_requests,
        ),
        tool_use_tokens: sum_optional(base.tool_use_tokens, new.tool_use_tokens),
        character_count: sum_optional(base.character_count, new.character_count),
        image_count: sum_optional(base.image_count, new.image_count),
        video_length_seconds: sum_decimal(base.video_length_seconds, new.video_length_seconds),
        audio_length_seconds: sum_decimal(base.audio_length_seconds, new.audio_length_seconds),
        query_count: sum_optional(base.query_count, new.query_count),
        cache_write_tokens,
        // LiteLLM mirrors these names; suppress the old alias when the canonical name arrives.
        cache_creation_tokens: if new.cache_write_tokens.is_some() {
            cache_write_tokens
        } else {
            sum_optional(base.cache_creation_tokens, new.cache_creation_tokens)
        },
        cached_tokens_details: new.cached_tokens_details.as_ref().map_or(
            base.cached_tokens_details,
            |value| {
                Some(combine_cached_tokens_details(
                    base.cached_tokens_details.as_ref(),
                    value,
                ))
            },
        ),
    }
}

pub fn usage_sum(usages: &[Usage]) -> Result<Usage, UsageSumError> {
    let mut combined = Usage::default();
    for usage in usages {
        combined.prompt_tokens =
            sum_field(combined.prompt_tokens, usage.prompt_tokens, "prompt_tokens")?;
        combined.completion_tokens = sum_field(
            combined.completion_tokens,
            usage.completion_tokens,
            "completion_tokens",
        )?;
        combined.total_tokens =
            sum_field(combined.total_tokens, usage.total_tokens, "total_tokens")?;
        if let Some(details) = usage.prompt_tokens_details.as_ref() {
            combined.prompt_tokens_details = Some(combine_prompt_tokens_details(
                combined.prompt_tokens_details.as_ref(),
                details,
            ));
        }
        if let Some(details) = usage.completion_tokens_details.as_ref() {
            let base = combined.completion_tokens_details.get_or_insert_default();
            sum_completion_details(base, details)?;
        }
        for (name, value) in &usage.numeric_extras {
            let merged = combined.numeric_extras.get(name).map_or_else(
                || Ok(value.clone()),
                |current| add_number(current, value, name),
            )?;
            combined.numeric_extras.insert(name.clone(), merged);
        }
    }
    Ok(combined)
}

impl Usage {
    pub fn from_cost_usage(value: &CostUsage) -> Self {
        let result = Self {
            prompt_tokens: Some(value.prompt_tokens),
            completion_tokens: Some(value.completion_tokens),
            total_tokens: Some(value.total_tokens),
            prompt_tokens_details: value.prompt_details.map(prompt_from_cost),
            completion_tokens_details: value.completion_details.map(completion_from_cost),
            numeric_extras: value
                .numeric_extras
                .iter()
                .map(|(name, value)| (name.clone(), UsageNumber::Decimal(*value)))
                .collect(),
        };
        result
    }

    pub fn to_cost_usage(&self) -> Result<CostUsage, UsageSumError> {
        let result = CostUsage {
            prompt_tokens: self.prompt_tokens.unwrap_or(0),
            completion_tokens: self.completion_tokens.unwrap_or(0),
            total_tokens: self.total_tokens.unwrap_or(0),
            prompt_details: self.prompt_tokens_details.as_ref().map(prompt_to_cost),
            completion_details: self
                .completion_tokens_details
                .as_ref()
                .map(completion_to_cost),
            cache_read_tokens: None,
            numeric_extras: self
                .numeric_extras
                .iter()
                .map(|(name, value)| decimal_number(value).map(|number| (name.clone(), number)))
                .collect::<Result<_, _>>()?,
        };
        Ok(result)
    }
}

fn sum_optional(current: Option<i64>, new: Option<i64>) -> Option<i64> {
    match (current, new) {
        (None, None) => None,
        (current, new) => Some(current.unwrap_or(0).saturating_add(new.unwrap_or(0))),
    }
}

fn sum_decimal(current: Option<Decimal>, new: Option<Decimal>) -> Option<Decimal> {
    match (current, new) {
        (None, None) => None,
        (current, new) => current
            .unwrap_or(Decimal::ZERO)
            .checked_add(new.unwrap_or(Decimal::ZERO)),
    }
}

fn sum_field(
    current: Option<i64>,
    new: Option<i64>,
    field: &str,
) -> Result<Option<i64>, UsageSumError> {
    match (current, new) {
        (None, None) => Ok(None),
        (current, new) => current
            .unwrap_or(0)
            .checked_add(new.unwrap_or(0))
            .map(Some)
            .ok_or_else(|| UsageSumError::IntegerOverflow {
                field: field.to_owned(),
            }),
    }
}

fn add_number(
    current: &UsageNumber,
    new: &UsageNumber,
    field: &str,
) -> Result<UsageNumber, UsageSumError> {
    match (current, new) {
        (UsageNumber::Integer(left), UsageNumber::Integer(right)) => left
            .checked_add(*right)
            .map(UsageNumber::Integer)
            .ok_or_else(|| UsageSumError::IntegerOverflow {
                field: field.to_owned(),
            }),
        (left, right) => decimal_number(left)
            .and_then(|left| decimal_number(right).and_then(|right| left.checked_add(right)))
            .map(UsageNumber::Decimal)
            .ok_or_else(|| UsageSumError::DecimalOverflow {
                field: field.to_owned(),
            }),
    }
}

fn decimal_number(value: &UsageNumber) -> Option<Decimal> {
    match value {
        UsageNumber::Integer(value) => Some(Decimal::from(*value)),
        UsageNumber::Decimal(value) => Some(*value),
        UsageNumber::Boolean(value) => Some(Decimal::from(u8::from(*value))),
    }
}

fn sum_completion_details(
    combined: &mut CompletionTokensDetails,
    new: &CompletionTokensDetails,
) -> Result<(), UsageSumError> {
    combined.text_tokens = sum_field(combined.text_tokens, new.text_tokens, "text_tokens")?;
    combined.audio_tokens = sum_field(combined.audio_tokens, new.audio_tokens, "audio_tokens")?;
    combined.image_tokens = sum_field(combined.image_tokens, new.image_tokens, "image_tokens")?;
    combined.video_tokens = sum_field(combined.video_tokens, new.video_tokens, "video_tokens")?;
    combined.reasoning_tokens = sum_field(
        combined.reasoning_tokens,
        new.reasoning_tokens,
        "reasoning_tokens",
    )?;
    combined.accepted_prediction_tokens = sum_field(
        combined.accepted_prediction_tokens,
        new.accepted_prediction_tokens,
        "accepted_prediction_tokens",
    )?;
    combined.rejected_prediction_tokens = sum_field(
        combined.rejected_prediction_tokens,
        new.rejected_prediction_tokens,
        "rejected_prediction_tokens",
    )?;
    Ok(())
}

fn prompt_from_cost(value: PromptDetails) -> PromptTokensDetails {
    let result = PromptTokensDetails {
        text_tokens: value.text_tokens,
        audio_tokens: value.audio_tokens,
        image_tokens: value.image_tokens,
        video_tokens: value.video_tokens,
        web_search_requests: value.web_search_requests,
        google_maps_grounding_requests: None,
        tool_use_tokens: None,
        character_count: None,
        image_count: None,
        video_length_seconds: None,
        audio_length_seconds: None,
        query_count: None,
        cached_tokens_details: None,
    };
    result
}
fn completion_from_cost(value: CompletionDetails) -> CompletionTokensDetails {
    let result = CompletionTokensDetails {
        text_tokens: value.text_tokens,
        audio_tokens: value.audio_tokens,
        image_tokens: value.image_tokens,
        video_tokens: value.video_tokens,
        reasoning_tokens: value.reasoning_tokens,
        audio_tokens: None,
        image_tokens: None,
        video_tokens: None,
        accepted_prediction_tokens: None,
        rejected_prediction_tokens: None,
    };
    result
}
fn prompt_to_cost(value: &PromptTokensDetails) -> PromptDetails {
    let result = PromptDetails {
        text_tokens: value.text_tokens,
        audio_tokens: value.audio_tokens,
        image_tokens: value.image_tokens,
        video_tokens: value.video_tokens,
        web_search_requests: value.web_search_requests,
    };
    result
}
fn completion_to_cost(value: &CompletionTokensDetails) -> CompletionDetails {
    let result = CompletionDetails {
        text_tokens: value.text_tokens,
        audio_tokens: value.audio_tokens,
        image_tokens: value.image_tokens,
        video_tokens: value.video_tokens,
        reasoning_tokens: value.reasoning_tokens,
    };
    result
}

mod tests;
