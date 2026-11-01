use serde::{Deserialize, Serialize};
use serde_json::Number;

use crate::baseline_history::error::BaselineHistoryError;
use crate::decisions::{Extra, Json};

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct CacheCreationTokenDetails {
    pub ephemeral_5m_input_tokens: Option<i64>,
    pub ephemeral_1h_input_tokens: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct PromptTokensDetails {
    #[serde(default)]
    pub text_tokens: Option<i64>,
    #[serde(default)]
    pub cached_tokens: Option<i64>,
    #[serde(default)]
    pub cache_creation_tokens: Option<i64>,
    #[serde(default)]
    pub cache_creation_token_details: Option<CacheCreationTokenDetails>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Usage {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    #[serde(default)]
    pub prompt_tokens_details: Option<PromptTokensDetails>,
    #[serde(flatten)]
    pub extra: Extra,
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
    details.extra.insert(
        "cache_write_tokens".to_owned(),
        Json::Number(Number::from(writes)),
    );
    let mut extra = usage.extra.clone();
    extra.insert(
        "cache_read_input_tokens".to_owned(),
        Json::Number(Number::from(read)),
    );
    extra.insert(
        "cache_creation_input_tokens".to_owned(),
        Json::Number(Number::from(writes)),
    );
    Ok(Usage {
        prompt_tokens: total,
        completion_tokens: usage.completion_tokens,
        total_tokens,
        prompt_tokens_details: Some(details),
        extra,
    })
}

mod tests;
