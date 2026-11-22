use std::collections::BTreeSet;
use std::sync::PoisonError;

use betterlite_error::proxy_rate_limit_error::{
    ProxyRateLimitProjectionError, RateLimitResetWindow,
};
use betterlite_error::rate_limit::ProxyRateLimitError;

use crate::limiter_records::{
    LimiterClock as _, PROJECT_ITPM_DESCRIPTOR_KEY, PROJECT_OTPM_DESCRIPTOR_KEY,
    ParallelSlotAcquisition, RateLimitDescriptor, RateLimitResponse, RateLimitType,
    RateLimitedModel, create_rate_limit_keys,
};
use crate::model_descriptors::ModelDescriptorBuilder;
use crate::pre_call_hook::RateLimitPreCallHook;
use crate::provider_attribution::{RateLimitErrorAttribution, handle_rate_limit_error};
use crate::request_stash::RequestStash;
use crate::token_reservations::TokenReservations;

pub(super) fn is_project_io(descriptor: &RateLimitDescriptor) -> bool {
    matches!(
        descriptor.key.as_str(),
        PROJECT_ITPM_DESCRIPTOR_KEY | PROJECT_OTPM_DESCRIPTOR_KEY
    )
}

pub(super) fn without_project_io(descriptors: &[RateLimitDescriptor]) -> Vec<RateLimitDescriptor> {
    descriptors
        .iter()
        .filter(|descriptor| !is_project_io(descriptor))
        .cloned()
        .collect()
}

pub(super) fn parallel_counter_keys(descriptors: &[RateLimitDescriptor]) -> Vec<String> {
    descriptors
        .iter()
        .filter(|descriptor| {
            descriptor
                .rate_limit
                .as_ref()
                .and_then(|limit| limit.max_parallel_requests)
                .is_some()
        })
        .map(|descriptor| {
            create_rate_limit_keys(
                &descriptor.key,
                &descriptor.value,
                RateLimitType::MaxParallelRequests,
            )
        })
        .collect()
}

pub(super) fn configured_tpm_limits(descriptors: &[RateLimitDescriptor]) -> Vec<i64> {
    descriptors
        .iter()
        .filter(|descriptor| !is_project_io(descriptor))
        .filter_map(|descriptor| descriptor.rate_limit.as_ref()?.tokens_per_unit)
        .collect()
}

pub(super) fn reserved_scopes(descriptors: &[RateLimitDescriptor]) -> Vec<(String, String)> {
    descriptors
        .iter()
        .filter(|descriptor| !is_project_io(descriptor))
        .filter(|descriptor| {
            descriptor
                .rate_limit
                .as_ref()
                .and_then(|limit| limit.tokens_per_unit)
                .is_some()
        })
        .map(|descriptor| (descriptor.key.clone(), descriptor.value.clone()))
        .collect()
}

pub(super) fn token_reservations<'deps>(
    hook: &'deps RateLimitPreCallHook,
    descriptors: &'deps ModelDescriptorBuilder,
    requested_model: Option<&'deps str>,
) -> TokenReservations<'deps> {
    TokenReservations {
        settlement: hook.assembly.settlement(),
        adjustments: hook.assembly.adjustments.as_ref(),
        descriptors,
        attribution: attribution(hook, requested_model),
    }
}

pub(super) fn project_rate_limit(
    hook: &RateLimitPreCallHook,
    response: &RateLimitResponse,
    descriptors: &[RateLimitDescriptor],
    requested_model: Option<&str>,
) -> Result<Option<ProxyRateLimitError>, ProxyRateLimitProjectionError> {
    let window = RateLimitResetWindow {
        now: hook.assembly.clock.current_time(),
        window_size: hook.assembly.settings.window_size,
    };
    handle_rate_limit_error(
        response,
        descriptors,
        window,
        &attribution(hook, requested_model),
    )
}

fn attribution<'deps>(
    hook: &'deps RateLimitPreCallHook,
    requested_model: Option<&'deps str>,
) -> RateLimitErrorAttribution<'deps> {
    RateLimitErrorAttribution {
        requested_model,
        native: hook.native.as_ref(),
        router: hook.router.as_deref(),
    }
}

pub(super) fn record_tag_scopes(stash: &RequestStash, descriptors: &[RateLimitDescriptor]) {
    let tags: BTreeSet<String> = descriptors
        .iter()
        .filter(|descriptor| descriptor.key == "tag")
        .filter(|descriptor| {
            descriptor
                .rate_limit
                .as_ref()
                .and_then(|limit| limit.tokens_per_unit)
                .is_some()
        })
        .map(|descriptor| descriptor.value.clone())
        .collect();
    let mut state = stash.state.lock().unwrap_or_else(PoisonError::into_inner);
    state.tpm_limited_tags = tags;
    drop(state);
}

pub(super) fn store_first_pass(
    stash: &RequestStash,
    response: &RateLimitResponse,
    slot: Option<ParallelSlotAcquisition>,
) {
    let mut state = stash.state.lock().unwrap_or_else(PoisonError::into_inner);
    state.rate_limit_response = Some(response.clone());
    if let Some(acquisition) = slot {
        state.parallel_slot = Some(acquisition);
    }
    drop(state);
}

pub(super) fn store_tpm_reservation(
    stash: &RequestStash,
    estimate: i64,
    model: Option<RateLimitedModel>,
    scopes: &[(String, String)],
    response: &RateLimitResponse,
) {
    let mut state = stash.state.lock().unwrap_or_else(PoisonError::into_inner);
    state.reserved_tokens = estimate;
    state.reserved_model = model;
    state.reserved_scopes = scopes.iter().cloned().collect();
    if let Some(stored) = state.rate_limit_response.as_mut() {
        stored.statuses.extend(response.statuses.iter().cloned());
    }
    drop(state);
}
