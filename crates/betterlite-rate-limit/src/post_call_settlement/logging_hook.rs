use betterlite_models::call_helpers::headers::AdditionalHeaders;

use crate::request_stash::RequestStashSlot;

use super::header_merge::merge_ratelimit_statuses_into_additional_headers;
use super::snapshot::rate_limit_statuses;

mod tests;

// the two header maps LiteLLM's mirror writes: the SLP's hidden_params.additional_headers and the response's
pub struct LoggingTargets<'targets> {
    pub standard_logging_object: Option<&'targets mut AdditionalHeaders>,
    pub response_hidden_params: Option<&'targets mut AdditionalHeaders>,
}

// LiteLLM's async_logging_hook leaves kwargs and the response untouched besides mirroring the stashed
// rate-limit statuses into whichever targets the caller holds; no statuses means no write at all
pub fn logging_hook(slot: &RequestStashSlot, call_id: Option<&str>, targets: LoggingTargets<'_>) {
    let stash = slot.for_call(call_id);
    let Some(statuses) = rate_limit_statuses(stash.as_deref()) else {
        return;
    };
    if let Some(logging) = targets.standard_logging_object {
        *logging = merge_ratelimit_statuses_into_additional_headers(logging, &statuses);
    }
    if let Some(response) = targets.response_hidden_params {
        *response = merge_ratelimit_statuses_into_additional_headers(response, &statuses);
    }
}
