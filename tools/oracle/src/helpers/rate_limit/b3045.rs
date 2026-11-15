use std::collections::{BTreeMap, BTreeSet};
use std::sync::PoisonError;

use betterlite_config::sections::rate_limit_usage::TokenRateLimitType;
use betterlite_models::call_helpers::metadata::MetadataMap;
use betterlite_rate_limit::limiter_records::{
    ModelGroupResolver, RateLimitedModel, ReservationAwareIncrement, ReservationBackend,
    ReservationWindowIdentity,
};
use betterlite_rate_limit::model_descriptors::ModelDescriptorBuilder;
use betterlite_rate_limit::reconciliation_operations::io_token::{
    IoTokenReconciliation, IoTokenResponse, io_token_reservation_ops,
};
use betterlite_rate_limit::reconciliation_operations::scope_targets::{
    ScopeMetadata, collect_tpm_scope_targets,
};
use betterlite_rate_limit::reconciliation_operations::success_event::{
    SuccessEvent, SuccessResponse, success_event_pipeline_ops,
};
use betterlite_rate_limit::request_stash::{RequestStashSlot, StashState};
use betterlite_rate_limit::usage_resolution::{ResponseAPIUsage, ResponseObject, Usage};
use betterlite_redis::cache_primitives::pipeline_increment::IncrementOperation;
use serde::Deserialize;
use serde_json::json;

use crate::helpers::OrHelperError as _;

struct GroupMap(BTreeMap<String, String>);

impl ModelGroupResolver for GroupMap {
    fn resolve(&self, model: &str) -> Option<String> {
        self.0.get(model).cloned()
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ResponseKind {
    #[default]
    Missing,
    Usage,
    ResponseApi,
    Mapping,
    WithUsage,
    Rerank,
    Unmodelled,
}

#[derive(Debug, Deserialize)]
struct TokenPair {
    input_tokens: u64,
    output_tokens: u64,
}

#[derive(Debug, Default, Deserialize)]
struct ResponseCase {
    #[serde(default)]
    kind: ResponseKind,
    #[serde(default)]
    usage: Option<Usage>,
    #[serde(default)]
    response_api_usage: Option<ResponseAPIUsage>,
    #[serde(default)]
    rerank_tokens: Option<TokenPair>,
    #[serde(default)]
    billed_units_total_tokens: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct ModelCase {
    requested: String,
    group: String,
}

#[derive(Debug, Deserialize)]
struct WindowCase {
    counter_key: String,
    window_start: String,
    backend: ReservationBackend,
}

#[derive(Debug, Default, Deserialize)]
struct StashCase {
    #[serde(default)]
    owner_call_id: Option<String>,
    #[serde(default)]
    reserved_tokens: i64,
    #[serde(default)]
    reserved_model: Option<ModelCase>,
    #[serde(default)]
    reserved_scopes: Vec<(String, String)>,
    #[serde(default)]
    itpm_reserved_tokens: i64,
    #[serde(default)]
    itpm_reserved_scopes: Vec<(String, String)>,
    #[serde(default)]
    itpm_reserved_windows: Vec<WindowCase>,
    #[serde(default)]
    otpm_reserved_tokens: i64,
    #[serde(default)]
    otpm_reserved_scopes: Vec<(String, String)>,
    #[serde(default)]
    otpm_reserved_windows: Vec<WindowCase>,
    #[serde(default)]
    reservation_released: bool,
    #[serde(default)]
    tpm_limited_tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct IoTokenCase {
    #[serde(default)]
    call_id: Option<String>,
    #[serde(default)]
    stash: Option<StashCase>,
    #[serde(default)]
    response: ResponseCase,
    #[serde(default)]
    combined_usage: Option<Usage>,
    window_size: i64,
}

#[derive(Debug, Default, Deserialize)]
struct ScopeTargetsCase {
    #[serde(default)]
    logging: ScopeMetadata,
    #[serde(default)]
    request_user: Option<String>,
    #[serde(default)]
    model_group: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct SuccessCase {
    #[serde(default)]
    call_id: Option<String>,
    #[serde(default)]
    stash: Option<StashCase>,
    #[serde(default)]
    response: ResponseCase,
    #[serde(default)]
    combined_usage: Option<Usage>,
    #[serde(default)]
    rate_limit_type: TokenRateLimitType,
    #[serde(default)]
    logging: ScopeMetadata,
    #[serde(default)]
    request_metadata: MetadataMap,
    #[serde(default)]
    request_user: Option<String>,
    #[serde(default)]
    model_group: Option<String>,
    #[serde(default)]
    groups: BTreeMap<String, String>,
    window_size: i64,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    IoTokenOps(IoTokenCase),
    ScopeTargets(ScopeTargetsCase),
    SuccessOps(SuccessCase),
}

fn model_value(case: &ModelCase) -> RateLimitedModel {
    RateLimitedModel {
        requested: case.requested.clone(),
        group: case.group.clone(),
    }
}

fn window_identity(case: &WindowCase) -> ReservationWindowIdentity {
    ReservationWindowIdentity {
        counter_key: case.counter_key.clone(),
        window_start: case.window_start.clone(),
        backend: case.backend,
    }
}

fn stash_slot(case: Option<&StashCase>) -> RequestStashSlot {
    let slot = RequestStashSlot::default();
    let Some(case) = case else {
        return slot;
    };
    let stash = slot.get_or_create();
    let state = StashState {
        owner_litellm_call_id: case.owner_call_id.clone(),
        rate_limit_response: None,
        parallel_slot: None,
        reserved_tokens: case.reserved_tokens,
        reserved_model: case.reserved_model.as_ref().map(model_value),
        reserved_scopes: case.reserved_scopes.iter().cloned().collect(),
        itpm_reserved_tokens: case.itpm_reserved_tokens,
        itpm_reserved_scopes: case.itpm_reserved_scopes.iter().cloned().collect(),
        itpm_reserved_windows: case
            .itpm_reserved_windows
            .iter()
            .map(window_identity)
            .collect(),
        otpm_reserved_tokens: case.otpm_reserved_tokens,
        otpm_reserved_scopes: case.otpm_reserved_scopes.iter().cloned().collect(),
        otpm_reserved_windows: case
            .otpm_reserved_windows
            .iter()
            .map(window_identity)
            .collect(),
        batch_enqueued_reservation: None,
        batch_tpd_refund_ops: Vec::new(),
        reservation_released: case.reservation_released,
        tpm_limited_tags: case.tpm_limited_tags.iter().cloned().collect(),
    };
    *stash.state.lock().unwrap_or_else(PoisonError::into_inner) = state;
    slot
}

fn response_object(case: &ResponseCase) -> ResponseObject {
    match case.kind {
        ResponseKind::Usage => ResponseObject::Usage(case.usage.clone().unwrap_or_default()),
        ResponseKind::ResponseApi => {
            ResponseObject::ResponseApi(case.response_api_usage.clone().unwrap_or_default())
        }
        ResponseKind::Mapping => ResponseObject::Mapping(case.usage.clone().unwrap_or_default()),
        ResponseKind::Missing
        | ResponseKind::WithUsage
        | ResponseKind::Rerank
        | ResponseKind::Unmodelled => ResponseObject::WithUsage(case.usage.clone()),
    }
}

fn reservation_value(operation: &ReservationAwareIncrement) -> serde_json::Value {
    json!({
        "key": operation.operation.key.clone(),
        "increment_value": operation.operation.increment_value,
        "ttl": operation.operation.ttl,
        "window_key": operation.window_key.clone(),
        "expected_window_start": operation.expected_window_start.clone(),
        "reservation_backend": operation.reservation_backend,
    })
}

fn reservation_values(mut operations: Vec<ReservationAwareIncrement>) -> Vec<serde_json::Value> {
    operations.sort_by_key(|operation| operation.operation.key.clone());
    operations.iter().map(reservation_value).collect()
}

fn increment_value(operation: &IncrementOperation) -> serde_json::Value {
    json!({
        "key": operation.key.clone(),
        "increment_value": operation.increment_value,
        "ttl": operation.ttl,
    })
}

fn target_values(targets: &[(String, String)]) -> Vec<serde_json::Value> {
    targets
        .iter()
        .map(|(key, value)| json!([key.clone(), value.clone()]))
        .collect()
}

fn io_token_answer(case: &IoTokenCase) -> crate::helpers::HelperResult {
    let slot = stash_slot(case.stash.as_ref());
    let object = response_object(&case.response);
    let response = if matches!(case.response.kind, ResponseKind::Rerank) {
        IoTokenResponse::Rerank {
            tokens: case
                .response
                .rerank_tokens
                .as_ref()
                .map(|pair| (pair.input_tokens, pair.output_tokens)),
            billed_total: case.response.billed_units_total_tokens,
        }
    } else {
        IoTokenResponse::Response(&object)
    };
    let reconciliation = IoTokenReconciliation {
        response,
        combined_usage: case.combined_usage.as_ref(),
        window_size: case.window_size,
    };
    let operations = io_token_reservation_ops(&slot, case.call_id.as_deref(), &reconciliation)
        .or_value_error()?;
    Ok(json!({"operations": reservation_values(operations)}))
}

fn scope_targets_answer(case: &ScopeTargetsCase) -> crate::helpers::HelperResult {
    let tags: BTreeSet<String> = case.tags.iter().cloned().collect();
    let targets = collect_tpm_scope_targets(
        &case.logging,
        case.request_user.as_deref(),
        case.model_group.as_deref(),
        &tags,
    );
    Ok(json!({"targets": target_values(&targets)}))
}

fn success_answer(case: &SuccessCase) -> crate::helpers::HelperResult {
    let slot = stash_slot(case.stash.as_ref());
    let models = ModelDescriptorBuilder {
        window_size: case.window_size,
        resolver: Box::new(GroupMap(case.groups.clone())),
    };
    let object = response_object(&case.response);
    let response = if matches!(case.response.kind, ResponseKind::Unmodelled) {
        SuccessResponse::Unmodelled
    } else {
        SuccessResponse::Modelled(&object)
    };
    let event = SuccessEvent {
        response,
        combined_usage: case.combined_usage.as_ref(),
        rate_limit_type: case.rate_limit_type,
        logging: &case.logging,
        request_metadata: &case.request_metadata,
        request_user: case.request_user.as_deref(),
        model_group: case.model_group.as_deref(),
        stash: &slot,
        call_id: case.call_id.as_deref(),
        models: &models,
        window_size: case.window_size,
    };
    let operations = success_event_pipeline_ops(&event).or_value_error()?;
    let increments: Vec<serde_json::Value> = operations.iter().map(increment_value).collect();
    Ok(json!({"operations": increments}))
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::IoTokenOps(case) => io_token_answer(case),
            Input::ScopeTargets(case) => scope_targets_answer(case),
            Input::SuccessOps(case) => success_answer(case),
        }
    }
}
