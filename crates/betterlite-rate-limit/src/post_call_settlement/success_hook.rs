use betterlite_core::api::batch_enqueued_reserve::response::BatchResponseView;
use betterlite_models::call_helpers::headers::AdditionalHeaders;

use crate::request_stash::{RequestStash, RequestStashSlot};

use super::batch_enqueued::BatchEnqueuedCall;
use super::header_merge::{merge_statuses_into, rate_limit_statuses};
use super::settlement::PostCallSettlement;

pub struct SuccessHook<'request> {
    pub stash: &'request RequestStashSlot,
    pub call_id: Option<&'request str>,
    pub response: Option<&'request BatchResponseView>,
}

impl PostCallSettlement<'_> {
    // LiteLLM's async_post_call_success_hook, with one guard per task: the slot release, the header merge and the
    // batch accounting each run on their own, and only the batch step can fail on this side
    pub async fn post_call_success_hook(
        &self,
        hook: &SuccessHook<'_>,
        response_headers: Option<&mut AdditionalHeaders>,
    ) {
        let slot_stash = hook.stash.for_call(hook.call_id);
        self.gauges
            .release_stashed_parallel_slot(slot_stash.as_deref(), false)
            .await;
        let header_stash = hook.stash.get();
        merge_response_headers(response_headers, header_stash.as_deref());
        let batch = BatchEnqueuedCall {
            stash: hook.stash,
            response: hook.response,
        };
        if let Err(error) = self.handle_batch_enqueued_post_call(&batch).await {
            tracing::warn!(%error, "Error in batch enqueued-token post-call hook");
        }
    }
}

// the post-call hook's header block reads the unfiltered stash and writes into the response's headers in place
fn merge_response_headers(target: Option<&mut AdditionalHeaders>, stash: Option<&RequestStash>) {
    let Some(target) = target else {
        return;
    };
    let Some(statuses) = rate_limit_statuses(stash) else {
        return;
    };
    merge_statuses_into(target, &statuses);
}
