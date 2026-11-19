use betterlite_core::api::batch_enqueued_reserve::response::BatchResponseView;
use betterlite_models::call_helpers::headers::AdditionalHeaders;

use crate::request_stash::{RequestStash, RequestStashSlot};

use super::header_merge::merge_statuses_into;
use super::snapshot::rate_limit_statuses;

pub struct SuccessHook<'request> {
    pub stash: &'request RequestStashSlot,
    pub call_id: Option<&'request str>,
    pub response: Option<&'request BatchResponseView>,
}

// the post-call hook's header block reads the unfiltered stash and writes into the response's headers in place
pub(super) fn merge_response_headers(
    headers: Option<&mut AdditionalHeaders>,
    stash: Option<&RequestStash>,
) {
    let Some(response_headers) = headers else {
        return;
    };
    let Some(statuses) = rate_limit_statuses(stash) else {
        return;
    };
    merge_statuses_into(response_headers, &statuses);
}
