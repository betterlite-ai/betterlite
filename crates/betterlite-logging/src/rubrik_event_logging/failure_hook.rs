use std::mem;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::rubrik_delivery::logger::RubrikLogger;
use crate::rubrik_moderation::context::{LoggingObjectStash, RequestData, text_field};
use crate::rubrik_moderation::error::ModifyResponseException;

use super::block_event::build_and_enqueue_block_event;

pub async fn async_post_call_failure_hook(
    logger: &RubrikLogger,
    request_data: &mut RequestData,
    exception: Option<&ModifyResponseException>,
    caller_metadata: &IndexMap<String, Json>,
) {
    let Some(exception) = exception else {
        return;
    };
    // Every registered Rubrik instance sees this hook; only the one whose guardrail raised the block may pop the stash.
    if exception.guardrail_name != logger.settings.guardrail_name {
        return;
    }
    let stash = mem::take(&mut request_data.logging_obj);
    let LoggingObjectStash::Present(Some(logging_obj)) = stash else {
        let call_id = text_field(&request_data.fields, "litellm_call_id")
            .ok()
            .flatten()
            .unwrap_or("None");
        let model = text_field(&request_data.fields, "model")
            .ok()
            .flatten()
            .unwrap_or("None");
        let user_id = text_field(caller_metadata, "user_api_key_user_id")
            .ok()
            .flatten()
            .unwrap_or("None");
        tracing::warn!(
            litellm_call_id = call_id,
            model,
            user_id,
            raising_guardrail = %exception.guardrail_name,
            "Rubrik: block exception without stashed logging_obj"
        );
        return;
    };
    build_and_enqueue_block_event(logger, &logging_obj, exception, caller_metadata).await;
}
