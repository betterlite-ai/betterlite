use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::argilla_logging::logger::RandomSource;
use crate::rubrik_delivery::logger::RubrikLogger;
use crate::rubrik_delivery::queue::append_and_maybe_flush;

use super::event::EventKind;
use super::prepare::prepare_log_payload;

pub async fn enqueue_log_event(
    logger: &RubrikLogger,
    random: &dyn RandomSource,
    call_details: &IndexMap<String, Json>,
    event: EventKind,
) {
    let payload =
        match prepare_log_payload(random, logger.settings.sampling_rate, call_details, event) {
            Ok(Some(payload)) => payload,
            Ok(None) => return,
            Err(error) => {
                tracing::error!(
                    %error,
                    event = %event,
                    "Rubrik logging hook failed. Skipping logging for this event."
                );
                return;
            }
        };
    append_and_maybe_flush(logger, Json::Object(payload)).await;
}
