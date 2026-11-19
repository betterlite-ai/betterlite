use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::argilla_logging::logger::RandomSource;
use crate::rubrik_delivery::logger::RubrikLogger;
use crate::rubrik_moderation::context::text_field;

use super::enqueue::enqueue_log_event;
use super::event::EventKind;
use super::fields::BLOCKED_FLAG;

pub async fn async_log_success_event(
    logger: &RubrikLogger,
    random: &dyn RandomSource,
    call_details: &IndexMap<String, Json>,
) {
    // The failure hook logs a block unsampled, so the pre-block response must not be logged here as well.
    if matches!(call_details.get(BLOCKED_FLAG), Some(Json::Bool(true))) {
        let call_id = text_field(call_details, "litellm_call_id")
            .ok()
            .flatten()
            .unwrap_or("None");
        tracing::debug!(
            litellm_call_id = call_id,
            "Rubrik: skipping success event for blocked request"
        );
        return;
    }
    enqueue_log_event(logger, random, call_details, EventKind::Success).await;
}
