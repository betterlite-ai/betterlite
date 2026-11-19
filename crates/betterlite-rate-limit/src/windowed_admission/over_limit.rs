use std::collections::BTreeMap;

use crate::limiter_records::{
    CacheCounterValue, RateLimitResponse, RateLimitStatus, RateLimitType, WindowKeyMetadata,
};
use crate::rate_limit_window_store::CounterValues;
use crate::rate_limit_window_store::counter_value::counter_as_int;

use super::{OK_CODE, OVER_LIMIT_CODE, WindowedAdmissionError};

pub fn is_cache_list_over_limit(
    keys_to_fetch: &[String],
    cache_values: &CounterValues,
    key_metadata: &BTreeMap<String, WindowKeyMetadata>,
    read_only: bool,
) -> Result<RateLimitResponse, WindowedAdmissionError> {
    if keys_to_fetch.len() != cache_values.len() || !cache_values.len().is_multiple_of(2) {
        return Err(unpaired(keys_to_fetch, cache_values));
    }
    let mut statuses = Vec::new();
    let mut overall_code = OK_CODE;
    let pairs = keys_to_fetch
        .chunks_exact(2)
        .zip(cache_values.chunks_exact(2));
    for (pair_keys, pair_values) in pairs {
        let Some(status) = pair_status(pair_keys, pair_values, key_metadata, read_only)? else {
            continue;
        };
        if status.code == OVER_LIMIT_CODE {
            overall_code = OVER_LIMIT_CODE;
        }
        statuses.push(status);
    }
    Ok(RateLimitResponse {
        overall_code: overall_code.to_owned(),
        statuses,
        reservation_windows: None,
    })
}

fn pair_status(
    pair_keys: &[String],
    pair_values: &[Option<CacheCounterValue>],
    key_metadata: &BTreeMap<String, WindowKeyMetadata>,
    read_only: bool,
) -> Result<Option<RateLimitStatus>, WindowedAdmissionError> {
    let (Some(window_key), Some(counter_key)) = (pair_keys.first(), pair_keys.get(1)) else {
        return Ok(None);
    };
    let Some(metadata) = key_metadata.get(window_key) else {
        return Err(WindowedAdmissionError::MissingWindowMetadata {
            window_key: window_key.clone(),
        });
    };
    let Some((current_limit, rate_limit_type)) = limit_for(counter_key, metadata) else {
        return Ok(None);
    };
    let counter_value = pair_values.get(1).and_then(Option::as_ref);
    let counter = counter_value.map(counter_as_int).transpose()?;
    let code = over_limit_code(counter, current_limit, read_only);
    Ok(Some(RateLimitStatus {
        code: code.to_owned(),
        current_limit,
        limit_remaining: counter.map_or(current_limit, |count| current_limit.saturating_sub(count)),
        rate_limit_type,
        descriptor_key: metadata.descriptor_key.clone(),
        descriptor_value: Some(metadata.descriptor_value.clone()),
    }))
}

fn limit_for(counter_key: &str, metadata: &WindowKeyMetadata) -> Option<(i64, RateLimitType)> {
    if counter_key.ends_with(":requests") {
        return metadata
            .requests_limit
            .map(|limit| (limit, RateLimitType::Requests));
    }
    if counter_key.ends_with(":tokens") {
        return metadata
            .tokens_limit
            .map(|limit| (limit, RateLimitType::Tokens));
    }
    None
}

fn over_limit_code(counter: Option<i64>, current_limit: i64, read_only: bool) -> &'static str {
    let Some(count) = counter else {
        return OK_CODE;
    };
    // read_only has not incremented yet, so the counter plus its pending request is over at the limit itself
    let over_limit = if read_only {
        count >= current_limit
    } else {
        count > current_limit
    };
    if over_limit { OVER_LIMIT_CODE } else { OK_CODE }
}

fn unpaired(keys_to_fetch: &[String], cache_values: &CounterValues) -> WindowedAdmissionError {
    WindowedAdmissionError::WindowPairCount {
        keys: keys_to_fetch.len(),
        values: cache_values.len(),
    }
}

mod tests;
