use std::sync::Arc;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::rubrik_delivery::logger::RubrikLogger;
use crate::rubrik_delivery::queue::append_and_maybe_flush;
use crate::rubrik_moderation::context::{LoggingObject, call_details, text_field};
use crate::rubrik_moderation::error::ModifyResponseException;

use super::block_payload::prepare_block_failure_payload;

pub async fn build_and_enqueue_block_event(
    logger: &RubrikLogger,
    logging_obj: &Arc<LoggingObject>,
    exception: &ModifyResponseException,
    caller_metadata: &IndexMap<String, Json>,
) {
    let details = call_details(Some(logging_obj));
    // The blocked flag stays on model_call_details: the deferred success handler may still read it.
    let call_id = text_field(&details, "litellm_call_id")
        .ok()
        .flatten()
        .unwrap_or("None");
    let payload = match prepare_block_failure_payload(&details, exception, caller_metadata) {
        Ok(payload) => payload,
        Err(error) => {
            tracing::error!(
                litellm_call_id = call_id,
                %error,
                "Rubrik: failed to build the blocked-tool payload; the event will not be logged"
            );
            return;
        }
    };
    append_and_maybe_flush(logger, Json::Object(payload)).await;
}
