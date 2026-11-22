use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_core::api::request_destination::RequestData;
use betterlite_core::routes::CallType;

use crate::limiter_assembly::CallTypeRateLimiter as _;
use crate::limiter_assembly::selection::{LimiterChoice, select_call_type_limiter};
use crate::limiter_records::{ParallelSlotAcquisition, RateLimitDescriptor, RateLimitResponse};
use crate::model_descriptors::ModelDescriptorBuilder;
use crate::pre_call_hook::common::{
    parallel_counter_keys, project_rate_limit, record_tag_scopes, store_first_pass,
    without_project_io,
};
use crate::pre_call_hook::{PreCallHookError, PreCallHookRequest, RateLimitPreCallHook};
use crate::request_scopes::body_text;
use crate::request_stash::{RequestStash, RequestStashSlot};
use crate::token_estimation::output_cap::CapEndpoint;
use crate::windowed_admission::SlotIdGenerator as _;
use crate::windowed_admission::{AdmissionRequest, OVER_LIMIT_CODE};

use super::reservation;

pub(super) struct FlowContext<'flow> {
    pub hook: &'flow RateLimitPreCallHook,
    pub auth: &'flow UserAPIKeyAuth,
    pub data: &'flow mut RequestData,
    pub descriptors: &'flow [RateLimitDescriptor],
    pub requested_model: Option<&'flow str>,
    pub stash: &'flow RequestStash,
    pub stash_slot: &'flow RequestStashSlot,
    pub call_type_name: Option<&'flow str>,
    pub endpoint: CapEndpoint,
    pub model: &'flow ModelDescriptorBuilder,
}

pub(super) async fn run(
    hook: &RateLimitPreCallHook,
    request: PreCallHookRequest<'_>,
) -> Result<(), PreCallHookError> {
    let PreCallHookRequest {
        auth,
        data,
        call_type,
        endpoint,
        stash: stash_slot,
    } = request;
    let stash = stash_slot.claim_for_call_id(body_text(data, "litellm_call_id"));
    if select_call_type_limiter(call_type) == LimiterChoice::Batch {
        return batch_pre_call(hook, auth, data, call_type, stash_slot).await;
    }
    let call_type_name = call_type_name(call_type);
    let builder = hook.request_scope_builder();
    let descriptors = hook
        .build_request_rate_limit_descriptors(auth, data, call_type_name.as_deref())
        .await?;
    record_tag_scopes(&stash, &descriptors);
    if descriptors.is_empty() {
        return Ok(());
    }
    let requested_model = body_text(data, "model").map(str::to_owned);
    first_pass(hook, &descriptors, &stash, requested_model.as_deref()).await?;
    let mut context = FlowContext {
        hook,
        auth,
        data,
        descriptors: &descriptors,
        requested_model: requested_model.as_deref(),
        stash: stash.as_ref(),
        stash_slot,
        call_type_name: call_type_name.as_deref(),
        endpoint,
        model: &builder.model,
    };
    let tpm = reservation::reserve_tokens(&mut context).await?;
    reservation::reserve_project_io(&mut context, &tpm).await
}

async fn batch_pre_call(
    hook: &RateLimitPreCallHook,
    auth: &UserAPIKeyAuth,
    data: &RequestData,
    call_type: CallType,
    stash: &RequestStashSlot,
) -> Result<(), PreCallHookError> {
    let Some(limiter) = hook.assembly.batch_limiter() else {
        return Err(PreCallHookError::MissingBatchLimiter { call_type });
    };
    limiter
        .async_pre_call_hook(auth, data, call_type, stash)
        .await
        .map_err(PreCallHookError::from_limiter_error)
}

async fn first_pass(
    hook: &RateLimitPreCallHook,
    descriptors: &[RateLimitDescriptor],
    stash: &RequestStash,
    requested_model: Option<&str>,
) -> Result<(), PreCallHookError> {
    let reservation = hook.assembly.settings.tpm_reservation_enabled;
    let counter_keys = parallel_counter_keys(descriptors);
    let slot_id = if counter_keys.is_empty() {
        None
    } else {
        Some(hook.assembly.admission.slot_ids.next_slot_id())
    };
    let first_pass_descriptors = if reservation {
        descriptors.to_vec()
    } else {
        without_project_io(descriptors)
    };
    let admission = AdmissionRequest {
        descriptors: first_pass_descriptors,
        parent_otel_span: None,
        read_only: false,
        skip_tpm_check: reservation,
        parallel_slot_id: slot_id.clone(),
    };
    let response = hook
        .assembly
        .admission
        .should_rate_limit(&admission)
        .await?;
    if response.overall_code == OVER_LIMIT_CODE {
        return reject_first_pass(hook, &response, descriptors, requested_model);
    }
    let slot = slot_id.map(|slot_id| ParallelSlotAcquisition {
        slot_id,
        counter_keys,
    });
    store_first_pass(stash, &response, slot);
    Ok(())
}

fn reject_first_pass(
    hook: &RateLimitPreCallHook,
    response: &RateLimitResponse,
    descriptors: &[RateLimitDescriptor],
    requested_model: Option<&str>,
) -> Result<(), PreCallHookError> {
    let Some(error) = project_rate_limit(hook, response, descriptors, requested_model)? else {
        return Ok(());
    };
    Err(PreCallHookError::RateLimited(Box::new(error)))
}

fn call_type_name(call_type: CallType) -> Option<String> {
    let text = serde_json::to_string(&call_type).ok()?;
    let trimmed = text.trim_matches('"').to_owned();
    Some(trimmed)
}
