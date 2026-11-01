use std::collections::BTreeMap;

use rust_decimal::Decimal;
use thiserror::Error;

mod combine;
mod projection;
mod tests;

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
    pub cached_tokens: Option<i64>,
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
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
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

pub fn usage_sum(usages: &[Usage]) -> Result<Usage, UsageSumError> {
    let mut combined = Usage::default();
    for usage in usages {
        combined.prompt_tokens = combined
            .prompt_tokens
            .checked_add(usage.prompt_tokens)
            .ok_or_else(|| UsageSumError::IntegerOverflow {
                field: "prompt_tokens".to_owned(),
            })?;
        combined.completion_tokens = combined
            .completion_tokens
            .checked_add(usage.completion_tokens)
            .ok_or_else(|| UsageSumError::IntegerOverflow {
                field: "completion_tokens".to_owned(),
            })?;
        combined.total_tokens = combined
            .total_tokens
            .checked_add(usage.total_tokens)
            .ok_or_else(|| UsageSumError::IntegerOverflow {
                field: "total_tokens".to_owned(),
            })?;
        if let Some(details) = usage.prompt_tokens_details.as_ref() {
            combined.prompt_tokens_details = Some(combine::combine_prompt_tokens_details(
                combined.prompt_tokens_details.as_ref(),
                details,
            )?);
        }
        if let Some(details) = usage.completion_tokens_details.as_ref() {
            let base = combined.completion_tokens_details.get_or_insert_default();
            combine::sum_completion_details(base, details)?;
        }
        for (name, value) in &usage.numeric_extras {
            let merged = combined.numeric_extras.get(name).map_or_else(
                || add_number(&UsageNumber::Integer(0), value, name),
                |current| add_number(current, value, name),
            )?;
            combined.numeric_extras.insert(name.clone(), merged);
        }
    }
    Ok(combined)
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
        (UsageNumber::Integer(left), UsageNumber::Boolean(right))
        | (UsageNumber::Boolean(right), UsageNumber::Integer(left)) => left
            .checked_add(i64::from(*right))
            .map(UsageNumber::Integer)
            .ok_or_else(|| UsageSumError::IntegerOverflow {
                field: field.to_owned(),
            }),
        (UsageNumber::Boolean(left), UsageNumber::Boolean(right)) => i64::from(*left)
            .checked_add(i64::from(*right))
            .map(UsageNumber::Integer)
            .ok_or_else(|| UsageSumError::IntegerOverflow {
                field: field.to_owned(),
            }),
        (left, right) => decimal_number(left)
            .checked_add(decimal_number(right))
            .map(UsageNumber::Decimal)
            .ok_or_else(|| UsageSumError::DecimalOverflow {
                field: field.to_owned(),
            }),
    }
}

fn decimal_number(number: &UsageNumber) -> Decimal {
    match number {
        UsageNumber::Integer(inner) => Decimal::from(*inner),
        UsageNumber::Decimal(inner) => *inner,
        UsageNumber::Boolean(inner) => Decimal::from(u8::from(*inner)),
    }
}
