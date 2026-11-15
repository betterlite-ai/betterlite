use std::collections::{BTreeMap, BTreeSet};
use std::sync::PoisonError;

use betterlite_config::sections::rate_limit_usage::TokenRateLimitType;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::call_helpers::metadata::MetadataMap;
use betterlite_models::decisions::Json;
use betterlite_redis::cache_primitives::pipeline_increment::IncrementOperation;

use crate::limiter_records::RateLimitedModel;
use crate::model_descriptors::ModelDescriptorBuilder;
// ast-grep-ignore: no-long-names
use crate::model_descriptors::key_owns_tpm::key_owns_tpm_from_request_metadata;
use crate::request_stash::RequestStashSlot;
use crate::token_adjustments::builders::{TokenReconciliation, build_tpm_reconciliation_ops};
use crate::usage_resolution::{
    ResponseObject, ResponseUsage, Usage, aggregate_only_total_tokens, get_total_tokens_from_usage,
    response_usage,
};

use super::scope_targets::{ScopeMetadata, collect_tpm_scope_targets};
use super::{ReconciliationError, token_count};

const INTERNAL_CALL_ORIGIN: &str = "internal_call_origin";
const COMPACTION_ORIGIN: &str = "autorouter_compaction";

#[derive(Clone, Copy, Debug)]
pub enum SuccessResponse<'request> {
    Modelled(&'request ResponseObject),
    Unmodelled,
}

pub struct SuccessEvent<'request> {
    pub response: SuccessResponse<'request>,
    pub combined_usage: Option<&'request Usage>,
    pub rate_limit_type: TokenRateLimitType,
    pub logging: &'request ScopeMetadata,
    pub request_metadata: &'request MetadataMap,
    pub request_user: Option<&'request str>,
    pub model_group: Option<&'request str>,
    pub stash: &'request RequestStashSlot,
    pub call_id: Option<&'request str>,
    pub models: &'request ModelDescriptorBuilder,
    pub window_size: i64,
}

pub fn success_event_pipeline_ops(
    event: &SuccessEvent<'_>,
) -> Result<Vec<IncrementOperation>, ReconciliationError> {
    if exempt_origin(event.request_metadata)? {
        return Ok(Vec::new());
    }
    let total_tokens = success_total_tokens(event);
    let snapshot = stash_snapshot(event.stash, event.call_id);
    let model = snapshot
        .reserved_model
        .clone()
        .or_else(|| event.models.rated_model(event.model_group));
    let mut targets = collect_tpm_scope_targets(
        event.logging,
        event.request_user,
        model.as_ref().map(|rated| rated.group.as_str()),
        &snapshot.tpm_limited_tags,
    );
    extend_reserved_agent_scopes(&mut targets, &snapshot.reserved_scopes);
    let charged = charged_targets(targets, event.request_metadata, model.as_ref())?;
    if snapshot.reserved_tokens > 0 && total_tokens < snapshot.reserved_tokens {
        tracing::debug!(
            reserved = snapshot.reserved_tokens,
            actual = total_tokens,
            release = snapshot.reserved_tokens.saturating_sub(total_tokens),
            "Releasing unused TPM budget on success"
        );
    }
    let reconciliation = TokenReconciliation {
        reserved_scopes: &snapshot.reserved_scopes,
        actual_tokens: total_tokens,
        reserved_tokens: snapshot.reserved_tokens,
        window_size: event.window_size,
    };
    build_tpm_reconciliation_ops(&charged, &reconciliation).map_err(ReconciliationError::from)
}

#[derive(Default)]
struct StashSnapshot {
    reserved_tokens: i64,
    reserved_model: Option<RateLimitedModel>,
    reserved_scopes: BTreeSet<(String, String)>,
    tpm_limited_tags: BTreeSet<String>,
}

fn stash_snapshot(slot: &RequestStashSlot, call_id: Option<&str>) -> StashSnapshot {
    let Some(stash) = slot.for_call(call_id) else {
        return StashSnapshot::default();
    };
    let state = stash.state.lock().unwrap_or_else(PoisonError::into_inner);
    StashSnapshot {
        reserved_tokens: state.reserved_tokens,
        reserved_model: state.reserved_model.clone(),
        reserved_scopes: state.reserved_scopes.clone(),
        tpm_limited_tags: state.tpm_limited_tags.clone(),
    }
}

fn extend_reserved_agent_scopes(
    targets: &mut Vec<(String, String)>,
    reserved_scopes: &BTreeSet<(String, String)>,
) {
    for scope in reserved_scopes {
        if matches!(scope.0.as_str(), "agent" | "agent_session") && !targets.contains(scope) {
            targets.push(scope.clone());
        }
    }
}

fn charged_targets(
    targets: Vec<(String, String)>,
    request_metadata: &MetadataMap,
    model: Option<&RateLimitedModel>,
) -> Result<Vec<(String, String)>, ReconciliationError> {
    let key_metadata = tpm_key_metadata(request_metadata);
    // ast-grep-ignore: no-long-names
    let owned = key_owns_tpm_from_request_metadata(&key_metadata, model)?;
    if !owned {
        return Ok(targets);
    }
    Ok(targets
        .into_iter()
        .filter(|target| target.0 != "model_per_team")
        .collect())
}

fn tpm_key_metadata(request_metadata: &MetadataMap) -> BTreeMap<String, JsonData> {
    let mut converted = BTreeMap::new();
    for field in ["user_api_key_metadata", "user_api_key_model_max_budget"] {
        if let Some(value) = request_metadata.get(field) {
            converted.insert(field.to_owned(), json_data(value));
        }
    }
    converted
}

fn json_data(value: &Json) -> JsonData {
    match value {
        Json::Null => JsonData::Null,
        Json::Bool(flag) => JsonData::Bool(*flag),
        Json::Number(number) => JsonData::Number(number.clone()),
        Json::Text(text) => JsonData::String(text.clone()),
        Json::List(items) => JsonData::Array(items.iter().map(json_data).collect()),
        Json::Object(entries) => JsonData::Object(
            entries
                .iter()
                .map(|(key, entry)| (key.clone(), json_data(entry)))
                .collect(),
        ),
    }
}

fn exempt_origin(request_metadata: &MetadataMap) -> Result<bool, ReconciliationError> {
    let Some(origin) = request_metadata.get(INTERNAL_CALL_ORIGIN) else {
        return Ok(false);
    };
    match origin {
        Json::Null => Ok(false),
        Json::Text(text) => Ok(!text.is_empty() && text.as_str() != COMPACTION_ORIGIN),
        _ => Err(ReconciliationError::MetadataField {
            field: INTERNAL_CALL_ORIGIN,
        }),
    }
}

fn success_total_tokens(event: &SuccessEvent<'_>) -> i64 {
    let total = get_total_tokens_from_usage(success_usage_source(event), event.rate_limit_type);
    if total > 0 {
        return token_count(total);
    }
    token_count(aggregate_only_total_tokens(success_usage_source(event)))
}

fn success_usage_source<'request>(
    event: &SuccessEvent<'request>,
) -> Option<ResponseUsage<'request>> {
    match event.response {
        SuccessResponse::Modelled(response) => response_usage(response),
        SuccessResponse::Unmodelled => event.combined_usage.map(ResponseUsage::Usage),
    }
}
