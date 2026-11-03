use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::baseline_history::error::BaselineHistoryError;
use crate::decisions::{Extra, Json};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum UsageError {
    #[error("invalid accounting data: {field} is outside the signed 64-bit integer range")]
    CounterOutOfRange { field: &'static str },
}

fn extra_counter(extra: &Extra, field: &'static str) -> Result<Option<i64>, UsageError> {
    let Some(Json::Number(number)) = extra.get(field) else {
        return Ok(None);
    };
    if number.as_str().contains(['.', 'e', 'E']) {
        return Ok(None);
    }
    number
        .as_i64()
        .map(Some)
        .ok_or(UsageError::CounterOutOfRange { field })
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct CacheCreationTokenDetails {
    pub ephemeral_5m_input_tokens: Option<i64>,
    pub ephemeral_1h_input_tokens: Option<i64>,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct RawPromptTokensDetails {
    #[serde(default)]
    text_tokens: Option<i64>,
    #[serde(default)]
    cached_tokens: Option<i64>,
    #[serde(default)]
    cache_creation_tokens: Option<i64>,
    #[serde(default)]
    cache_creation_token_details: Option<CacheCreationTokenDetails>,
    #[serde(default)]
    cache_write_tokens: Option<i64>,
    #[serde(flatten)]
    extra: Extra,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawPromptTokensDetails")]
pub struct PromptTokensDetails {
    pub text_tokens: Option<i64>,
    pub cached_tokens: Option<i64>,
    pub cache_creation_tokens: Option<i64>,
    pub cache_creation_token_details: Option<CacheCreationTokenDetails>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<i64>,
    #[serde(flatten)]
    pub extra: Extra,
}

impl TryFrom<RawPromptTokensDetails> for PromptTokensDetails {
    type Error = UsageError;

    fn try_from(raw: RawPromptTokensDetails) -> Result<Self, Self::Error> {
        let nested_creation = extra_counter(&raw.extra, "cache_creation_input_tokens")?;
        let mirrored = raw
            .cache_write_tokens
            .or(raw.cache_creation_tokens)
            .or(nested_creation);
        Ok(Self {
            text_tokens: raw.text_tokens,
            cached_tokens: raw.cached_tokens,
            cache_creation_tokens: mirrored,
            cache_creation_token_details: raw.cache_creation_token_details,
            cache_write_tokens: mirrored,
            extra: raw.extra,
        })
    }
}

#[derive(Clone, Debug, Deserialize)]
struct RawUsage {
    prompt_tokens: i64,
    completion_tokens: i64,
    total_tokens: i64,
    #[serde(default)]
    prompt_tokens_details: Option<RawPromptTokensDetails>,
    #[serde(default)]
    cache_read_input_tokens: Option<i64>,
    #[serde(default)]
    cache_creation_input_tokens: Option<i64>,
    #[serde(flatten)]
    extra: Extra,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawUsage")]
pub struct Usage {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    #[serde(default)]
    pub prompt_tokens_details: Option<PromptTokensDetails>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_input_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_creation_input_tokens: Option<i64>,
    #[serde(flatten)]
    pub extra: Extra,
}

impl TryFrom<RawUsage> for Usage {
    type Error = UsageError;

    fn try_from(raw: RawUsage) -> Result<Self, Self::Error> {
        let mut details = raw
            .prompt_tokens_details
            .map(PromptTokensDetails::try_from)
            .transpose()?;
        let cache_hit = extra_counter(&raw.extra, "prompt_cache_hit_tokens")?;
        if let Some(cached) = raw.cache_read_input_tokens.or(cache_hit) {
            details.get_or_insert_default().cached_tokens = Some(cached);
        }
        if let Some(creation) = raw.cache_creation_input_tokens {
            let prompt_details = details.get_or_insert_default();
            prompt_details.cache_creation_tokens = Some(creation);
            prompt_details.cache_write_tokens = Some(creation);
        }
        Ok(Self {
            prompt_tokens: raw.prompt_tokens,
            completion_tokens: raw.completion_tokens,
            total_tokens: raw.total_tokens,
            prompt_tokens_details: details,
            cache_read_input_tokens: raw.cache_read_input_tokens,
            cache_creation_input_tokens: raw.cache_creation_input_tokens,
            extra: raw.extra,
        })
    }
}

pub(crate) fn is_usage_complete(maybe_usage: Option<&Usage>) -> bool {
    let Some(usage) = maybe_usage else {
        return false;
    };
    if usage.prompt_tokens < 0 || usage.completion_tokens < 0 {
        return false;
    }
    let Some(details) = &usage.prompt_tokens_details else {
        return false;
    };
    let Some(text) = details.text_tokens.filter(|value| *value >= 0) else {
        return false;
    };
    let Some(cached) = details.cached_tokens.filter(|value| *value >= 0) else {
        return false;
    };
    let Some(creation) = details.cache_creation_tokens.filter(|value| *value >= 0) else {
        return false;
    };
    is_usage_total_consistent(usage, text, cached, creation)
        && write_split_matches_creation(details, creation)
}

fn is_usage_total_consistent(usage: &Usage, text: i64, cached: i64, creation: i64) -> bool {
    let Some(prompt_total) = usage.prompt_tokens.checked_add(usage.completion_tokens) else {
        return false;
    };
    if usage.total_tokens != prompt_total {
        return false;
    }
    let Some(detail_sum) = text
        .checked_add(cached)
        .and_then(|sum| sum.checked_add(creation))
    else {
        return false;
    };
    detail_sum == usage.prompt_tokens
}

fn write_split_matches_creation(details: &PromptTokensDetails, creation: i64) -> bool {
    if creation == 0 {
        return true;
    }
    let Some(split) = &details.cache_creation_token_details else {
        return false;
    };
    let (Some(write_short_ttl), Some(write_long_ttl)) = (
        split.ephemeral_5m_input_tokens,
        split.ephemeral_1h_input_tokens,
    ) else {
        return false;
    };
    write_short_ttl >= 0
        && write_long_ttl >= 0
        && write_short_ttl.checked_add(write_long_ttl) == Some(creation)
}

pub(crate) fn usage_with_cache(
    usage: &Usage,
    total: i64,
    read: i64,
    write_short_ttl: i64,
    write_long_ttl: i64,
) -> Result<Usage, BaselineHistoryError> {
    let writes = write_short_ttl
        .checked_add(write_long_ttl)
        .ok_or(BaselineHistoryError::CounterOverflow)?;
    let text_tokens = total
        .checked_sub(read)
        .and_then(|value| value.checked_sub(writes))
        .ok_or(BaselineHistoryError::CounterOverflow)?;
    let total_tokens = total
        .checked_add(usage.completion_tokens)
        .ok_or(BaselineHistoryError::CounterOverflow)?;
    let mut details = usage.prompt_tokens_details.clone().unwrap_or_default();
    details.text_tokens = Some(text_tokens);
    details.cached_tokens = Some(read);
    details.cache_creation_tokens = Some(writes);
    details.cache_creation_token_details = Some(CacheCreationTokenDetails {
        ephemeral_5m_input_tokens: Some(write_short_ttl),
        ephemeral_1h_input_tokens: Some(write_long_ttl),
    });
    details.cache_write_tokens = Some(writes);
    Ok(Usage {
        prompt_tokens: total,
        completion_tokens: usage.completion_tokens,
        total_tokens,
        prompt_tokens_details: Some(details),
        cache_read_input_tokens: Some(read),
        cache_creation_input_tokens: Some(writes),
        extra: usage.extra.clone(),
    })
}

mod tests;
