use betterlite_auth::auth_limit_inputs::key_limits::{
    LimitQuery, ModelLimits, get_key_model_rpm_limit, get_key_model_tpm_limit,
};
use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::Object as JsonMap;

use super::Call;
use super::error::LegacyAdmissionError;
use super::facts::{GLOBAL_KEY, NONE_TEXT, UNLIMITED};
use crate::rate_limiters::legacy_rate_limit_counters::LegacyRateLimitHandler;
use crate::rate_limiters::legacy_rate_limit_counters::limits::{
    DimensionLimits, KeyLimitCheck, RateLimitScope,
};
use crate::rate_limiters::legacy_rate_limit_counters::usage_cache::{
    LegacyCacheValue, MinuteCounter,
};

// InMemoryCache.default_ttl: what DualCache.async_increment_cache gives the key when no ttl is live
const GLOBAL_TTL_SECONDS: u64 = 600;

// async_get_cache answering None stands for the first request in the window
const MISSING_GLOBAL_COUNT: i64 = 1;

#[must_use]
pub(super) fn global_cap(metadata: &JsonMap) -> Option<i64> {
    metadata.get(GLOBAL_KEY).and_then(JsonValue::as_i64)
}

// step 3: compare the global counter with the cap, raise at the cap, and otherwise count this request
pub(super) async fn global(
    handler: &LegacyRateLimitHandler,
    cap: i64,
    model: Option<&str>,
) -> Result<(), LegacyAdmissionError> {
    let read = handler
        .internal_usage_cache
        .batch_get(&[Some(String::from(GLOBAL_KEY))], None)
        .await?;
    let current = global_counter(read.as_deref());
    if current >= cap {
        let details = format!("Hit Global Limit: Limit={cap}, current: {current}");
        let error = handler.raise_rate_limit_error(Some(&details), None, model);
        return Err(LegacyAdmissionError::from(error));
    }
    // async_increment_cache on a local-only key: the whole number goes up by one under the default ttl
    let entries = [(
        String::from(GLOBAL_KEY),
        LegacyCacheValue::Count(current.saturating_add(1)),
    )];
    handler
        .internal_usage_cache
        .batch_set(&entries, GLOBAL_TTL_SECONDS, None, true)
        .await?;
    Ok(())
}

// a bare count is what the global step stores; any other shape, or a wrong-length answer, counts as missing
#[must_use]
fn global_counter(answer: Option<&[Option<LegacyCacheValue>]>) -> i64 {
    let Some(values) = answer else {
        return MISSING_GLOBAL_COUNT;
    };
    match values {
        [None] => MISSING_GLOBAL_COUNT,
        [Some(LegacyCacheValue::Count(current))] => *current,
        other => {
            tracing::error!(
                "LiteLLM Cache: the global counter read answered {other:?} instead of one whole number"
            );
            MISSING_GLOBAL_COUNT
        }
    }
}

// step 5: the key's own max_parallel_requests, tpm and rpm
pub(super) async fn key(
    handler: &LegacyRateLimitHandler,
    call: &mut Call<'_>,
) -> Result<(), LegacyAdmissionError> {
    let Some(counter_id) = call.ids.key.clone() else {
        return Ok(());
    };
    let check = KeyLimitCheck {
        limits: DimensionLimits {
            max_parallel_requests: call.facts.max_parallel_requests,
            tpm_limit: call.facts.tpm_limit,
            rpm_limit: call.facts.rpm_limit,
        },
        current: minute_counter(call.objects.request_count_api_key),
        request_count_api_key: counter_id,
        rate_limit_type: RateLimitScope::Key,
        requested_model: call.model,
        parent_otel_span: None,
    };
    handler
        .check_key_in_limits(check, &mut call.counters)
        .await?;
    Ok(())
}

// step 6: the per-model limits, then the remaining budget the hook writes back into the request metadata
pub(super) async fn model(
    handler: &LegacyRateLimitHandler,
    call: &mut Call<'_>,
    metadata: &mut JsonMap,
) -> Result<(), LegacyAdmissionError> {
    let query = LimitQuery {
        model_name: call.model,
        catalog: None,
    };
    let tpm_limits = get_key_model_tpm_limit(call.principal, &query).unwrap_or_default();
    let rpm_limits = get_key_model_rpm_limit(call.principal, &query).unwrap_or_default();
    if tpm_limits.is_none() && rpm_limits.is_none() {
        return Ok(());
    }
    let tpm_for_model = limit_for(tpm_limits.as_ref(), call.model);
    let rpm_for_model = limit_for(rpm_limits.as_ref(), call.model);
    let check = KeyLimitCheck {
        limits: DimensionLimits {
            max_parallel_requests: UNLIMITED,
            tpm_limit: enforced(tpm_for_model),
            rpm_limit: enforced(rpm_for_model),
        },
        current: minute_counter(call.objects.request_count_api_key_model),
        request_count_api_key: call.ids.key_model_text.clone(),
        rate_limit_type: RateLimitScope::ModelPerKey,
        requested_model: call.model,
        parent_otel_span: None,
    };
    let counters = handler
        .check_key_in_limits(check, &mut call.counters)
        .await?;
    let model_text = call.model.unwrap_or(NONE_TEXT);
    metadata.insert(
        format!("litellm-key-remaining-tokens-{model_text}"),
        remaining(tpm_for_model, counters.current_tpm),
    );
    metadata.insert(
        format!("litellm-key-remaining-requests-{model_text}"),
        remaining(rpm_for_model, counters.current_rpm),
    );
    Ok(())
}

// Python's `if map:` then `map.get(model)`: a missing model or map entry is unlimited
#[must_use]
fn limit_for(limits: Option<&ModelLimits>, model: Option<&str>) -> Option<i64> {
    let model = model?;
    limits
        .filter(|limits| !limits.is_empty())
        .and_then(|limits| limits.get(model).copied())
}

// Python's `limit or sys.maxsize`: an absent or zero per-model limit is unlimited
#[must_use]
fn enforced(limit: Option<i64>) -> i64 {
    limit.filter(|limit| *limit != 0).unwrap_or(UNLIMITED)
}

// the remaining-limit math uses is-not-None, so a zero limit still yields `0 - current`
#[must_use]
fn remaining(limit: Option<i64>, current: i64) -> JsonValue {
    limit.map_or(JsonValue::Null, |limit| {
        JsonValue::from(limit.saturating_sub(current))
    })
}

// step 7: the user, gated on is-not-None so an empty user id is checked
pub(super) async fn user(
    handler: &LegacyRateLimitHandler,
    call: &mut Call<'_>,
) -> Result<(), LegacyAdmissionError> {
    let Some(counter_id) = call.ids.user.clone() else {
        return Ok(());
    };
    let check = KeyLimitCheck {
        limits: DimensionLimits {
            max_parallel_requests: UNLIMITED,
            tpm_limit: call.facts.user_tpm_limit,
            rpm_limit: call.facts.user_rpm_limit,
        },
        current: minute_counter(call.objects.request_count_user_id),
        request_count_api_key: counter_id,
        rate_limit_type: RateLimitScope::User,
        requested_model: call.model,
        parent_otel_span: None,
    };
    handler
        .check_key_in_limits(check, &mut call.counters)
        .await?;
    Ok(())
}

// step 8: the team, gated on is-not-None
pub(super) async fn team(
    handler: &LegacyRateLimitHandler,
    call: &mut Call<'_>,
) -> Result<(), LegacyAdmissionError> {
    let Some(counter_id) = call.ids.team.clone() else {
        return Ok(());
    };
    let check = KeyLimitCheck {
        limits: DimensionLimits {
            max_parallel_requests: UNLIMITED,
            tpm_limit: call.facts.team_tpm_limit,
            rpm_limit: call.facts.team_rpm_limit,
        },
        current: minute_counter(call.objects.request_count_team_id),
        request_count_api_key: counter_id,
        rate_limit_type: RateLimitScope::Team,
        requested_model: call.model,
        parent_otel_span: None,
    };
    handler
        .check_key_in_limits(check, &mut call.counters)
        .await?;
    Ok(())
}

// step 9: the end user, gated on truthiness, so an empty id is read from the cache and never checked
pub(super) async fn end_user(
    handler: &LegacyRateLimitHandler,
    call: &mut Call<'_>,
) -> Result<(), LegacyAdmissionError> {
    let Some(end_user_id) = call.facts.end_user_id.as_deref() else {
        return Ok(());
    };
    if end_user_id.is_empty() {
        return Ok(());
    }
    let Some(counter_id) = call.ids.end_user.clone() else {
        return Ok(());
    };
    let check = KeyLimitCheck {
        limits: DimensionLimits {
            max_parallel_requests: UNLIMITED,
            tpm_limit: call.facts.end_user_tpm_limit,
            rpm_limit: call.facts.end_user_rpm_limit,
        },
        current: minute_counter(call.objects.request_count_end_user_id),
        request_count_api_key: counter_id,
        rate_limit_type: RateLimitScope::Customer,
        requested_model: call.model,
        parent_otel_span: None,
    };
    handler
        .check_key_in_limits(check, &mut call.counters)
        .await?;
    Ok(())
}

// a minute entry holds a counter object; a bare number there is a broken writer, which B3047's reads treat as missing
#[must_use]
fn minute_counter(value: Option<LegacyCacheValue>) -> Option<MinuteCounter> {
    match value {
        Some(LegacyCacheValue::Counter(counter)) => Some(counter),
        Some(LegacyCacheValue::Count(count)) => {
            tracing::error!("LiteLLM Cache: the minute counter holds the bare number {count}");
            None
        }
        None => None,
    }
}
