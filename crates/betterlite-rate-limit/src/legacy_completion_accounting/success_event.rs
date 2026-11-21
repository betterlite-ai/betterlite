use std::collections::BTreeMap;

use betterlite_auth::auth_limit_inputs::deployment::DeploymentLimitCatalog;
use betterlite_auth::auth_limit_inputs::key_limits::{
    LimitQuery, get_key_model_rpm_limit, get_key_model_tpm_limit,
};
use betterlite_auth::resolved_key::auth::{AuthInput, ParentOtelSpan, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;

use crate::rate_limiters::legacy_rate_limit_counters::usage_cache::{
    LegacyCacheValue, MinuteCounter,
};
use crate::rate_limiters::legacy_rate_limit_counters::{
    COUNTER_TTL_SECONDS, LegacyRateLimitHandler,
};

use super::counter_cache::{CounterCache, GLOBAL_PARALLEL_KEY, read_counter, released_requests};
use super::error::AccountingError;
use super::response_tokens::{CompletionResponse, response_total_tokens};

// metadata["user_api_key"]: LiteLLM indexes it, so a missing key fails the hook while a None only skips the key counter
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UserApiKeyField {
    Missing,
    Null,
    Key(String),
}

// the litellm_params.metadata fields the success hook reads; user_api_key_metadata's absent and null are already {}
#[derive(Clone, Debug)]
pub struct SuccessMetadata {
    pub global_max_parallel_requests: Option<i64>,
    pub user_api_key: UserApiKeyField,
    pub user_api_key_user_id: Option<String>,
    pub user_api_key_team_id: Option<String>,
    pub user_api_key_model_max_budget: Option<BTreeMap<String, JsonData>>,
    pub user_api_key_metadata: BTreeMap<String, JsonData>,
    pub user_api_key_team_metadata: Option<BTreeMap<String, JsonData>>,
}

// one success callback; the caller extracts the model group (B3891), the parent span and the router's limit catalog
pub struct SuccessEvent<'event> {
    pub metadata: SuccessMetadata,
    pub end_user: Option<String>,
    pub model_group: Option<String>,
    pub response: CompletionResponse,
    pub limit_catalog: Option<&'event (dyn DeploymentLimitCatalog + Sync)>,
    pub parent_otel_span: Option<&'event ParentOtelSpan>,
}

// what every counter update in one success call shares
struct SuccessPass<'pass> {
    counters: &'pass dyn CounterCache,
    total_tokens: i64,
    precise_minute: String,
    parent_otel_span: Option<&'pass ParentOtelSpan>,
}

// async_log_success_event: LiteLLM's catch-all turns every failure into a verbose print
pub async fn log_success_event(
    handler: &LegacyRateLimitHandler,
    counters: &dyn CounterCache,
    event: &SuccessEvent<'_>,
) {
    if let Err(error) = account_success(handler, counters, event).await {
        handler.print_verbose(&error.to_string());
    }
}

async fn account_success(
    handler: &LegacyRateLimitHandler,
    counters: &dyn CounterCache,
    event: &SuccessEvent<'_>,
) -> Result<(), AccountingError> {
    handler.print_verbose("INSIDE parallel request limiter ASYNC SUCCESS LOGGING");
    let metadata = &event.metadata;
    let user_api_key = match &metadata.user_api_key {
        UserApiKeyField::Missing => return Err(AccountingError::MissingUserApiKey),
        UserApiKeyField::Null => None,
        UserApiKeyField::Key(key) => Some(key.as_str()),
    };
    let span = event.parent_otel_span;
    if metadata.global_max_parallel_requests.is_some() {
        counters
            .increment(GLOBAL_PARALLEL_KEY, -1, span, true)
            .await
            .map_err(|source| AccountingError::Cache {
                key: GLOBAL_PARALLEL_KEY.to_owned(),
                source,
            })?;
    }
    let total_tokens = response_total_tokens(&event.response);
    let pass = SuccessPass {
        counters,
        total_tokens,
        precise_minute: handler.precise_minute(),
        parent_otel_span: span,
    };
    let values = settled_counters(handler, &pass, user_api_key, event).await?;
    handler
        .internal_usage_cache
        .batch_set(&values, COUNTER_TTL_SECONDS, span, false)
        .await
        .map_err(AccountingError::BatchWrite)
}

// the new counters in LiteLLM's block order: key, model group, user, team, end user
async fn settled_counters(
    handler: &LegacyRateLimitHandler,
    pass: &SuccessPass<'_>,
    user_api_key: Option<&str>,
    event: &SuccessEvent<'_>,
) -> Result<Vec<(String, LegacyCacheValue)>, AccountingError> {
    let minute = &pass.precise_minute;
    let key_fallback = MinuteCounter {
        current_requests: 1,
        current_tpm: 0,
        current_rpm: 0,
    };
    let mut values = Vec::new();
    if let Some(api_key) = user_api_key {
        let key = format!("{api_key}::{minute}::request_count");
        values.push(settled(handler, pass, key, key_fallback).await?);
    }
    if let Some((api_key, group)) = model_counter_owner(user_api_key, event)? {
        let key = format!("{api_key}::{group}::{minute}::request_count");
        values.push(settled(handler, pass, key, key_fallback).await?);
    }
    let metadata = &event.metadata;
    let entities = [
        metadata.user_api_key_user_id.as_deref(),
        metadata.user_api_key_team_id.as_deref(),
        event.end_user.as_deref(),
    ];
    // LiteLLM seeds a missing entity counter with the tokens and then adds them again
    let entity_fallback = MinuteCounter {
        current_requests: 1,
        current_tpm: pass.total_tokens,
        current_rpm: 1,
    };
    for entity in entities.into_iter().flatten() {
        let key = format!("{entity}::{minute}::request_count");
        values.push(settled(handler, pass, key, entity_fallback).await?);
    }
    Ok(values)
}

async fn settled(
    handler: &LegacyRateLimitHandler,
    pass: &SuccessPass<'_>,
    key: String,
    fallback: MinuteCounter,
) -> Result<(String, LegacyCacheValue), AccountingError> {
    let current = read_counter(pass.counters, &key, pass.parent_otel_span)
        .await?
        .unwrap_or(fallback);
    let new_val = MinuteCounter {
        current_requests: released_requests(current.current_requests, &key)?,
        current_tpm: current
            .current_tpm
            .checked_add(pass.total_tokens)
            .ok_or_else(|| AccountingError::Overflow { key: key.clone() })?,
        current_rpm: current.current_rpm,
    };
    handler.print_verbose(&format!(
        "updated_value in success call: {{current_requests: {}, current_tpm: {}, current_rpm: {}}}, precise_minute: {}",
        new_val.current_requests, new_val.current_tpm, new_val.current_rpm, pass.precise_minute
    ));
    Ok((key, LegacyCacheValue::Counter(new_val)))
}

// the model group counter exists only for a key with some per-model limit; the lookups run whenever a group is set
fn model_counter_owner<'event>(
    user_api_key: Option<&'event str>,
    event: &'event SuccessEvent<'_>,
) -> Result<Option<(&'event str, &'event str)>, AccountingError> {
    let Some(group) = event.model_group.as_deref() else {
        return Ok(None);
    };
    let metadata = &event.metadata;
    let lookup_found = model_limit_found(metadata, user_api_key, group, event.limit_catalog)?;
    let Some(api_key) = user_api_key else {
        return Ok(None);
    };
    let limited = metadata
        .user_api_key_metadata
        .contains_key("model_rpm_limit")
        || metadata
            .user_api_key_metadata
            .contains_key("model_tpm_limit")
        || metadata.user_api_key_model_max_budget.is_some()
        || lookup_found;
    Ok(limited.then_some((api_key, group)))
}

// get_key_model_tpm_limit, then get_key_model_rpm_limit, on a key auth value of only these four fields
fn model_limit_found(
    metadata: &SuccessMetadata,
    user_api_key: Option<&str>,
    group: &str,
    catalog: Option<&(dyn DeploymentLimitCatalog + Sync)>,
) -> Result<bool, AccountingError> {
    let mut fields = BTreeMap::from([
        (
            String::from("api_key"),
            user_api_key.map_or(JsonData::Null, |key| JsonData::String(key.to_owned())),
        ),
        (
            String::from("metadata"),
            JsonData::Object(metadata.user_api_key_metadata.clone()),
        ),
    ]);
    if let Some(budget) = &metadata.user_api_key_model_max_budget {
        fields.insert(
            String::from("model_max_budget"),
            JsonData::Object(budget.clone()),
        );
    }
    if let Some(team_metadata) = &metadata.user_api_key_team_metadata {
        fields.insert(
            String::from("team_metadata"),
            JsonData::Object(team_metadata.clone()),
        );
    }
    let auth =
        UserAPIKeyAuth::construct(AuthInput::Dict(fields)).map_err(AccountingError::KeyAuth)?;
    let mut query = LimitQuery {
        model_name: Some(group),
        catalog: None,
    };
    if let Some(source) = catalog {
        query.catalog = Some(source);
    }
    let tpm_limit = get_key_model_tpm_limit(&auth, &query).map_err(AccountingError::ModelLimit)?;
    let rpm_limit = get_key_model_rpm_limit(&auth, &query).map_err(AccountingError::ModelLimit)?;
    Ok(tpm_limit.is_some() || rpm_limit.is_some())
}
