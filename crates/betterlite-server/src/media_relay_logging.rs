pub mod deepgram;
pub mod fal;

use betterlite_core::api::media_relay_logging::spend::MediaSpend;
use betterlite_logging::passthrough_stream_logging::object::StreamLoggingObject;

// The spend logger reads the model and cost off the shared logging object; the kwargs carry them too.
pub(crate) fn write_spend(logging_obj: &StreamLoggingObject, spend: &MediaSpend) {
    let entries = spend.entries();
    logging_obj.edit(&mut |state| {
        state.model = spend.model.clone();
        for (key, value) in &entries {
            state.model_call_details.insert(key.clone(), value.clone());
        }
    });
}

#[path = "media_relay_logging/tests/deepgram_adjustments.rs"]
mod deepgram_adjustments;
#[path = "media_relay_logging/tests/deepgram_listen.rs"]
mod deepgram_listen;
#[path = "media_relay_logging/tests/deepgram_routes.rs"]
mod deepgram_routes;
#[path = "media_relay_logging/tests/deepgram_warning.rs"]
mod deepgram_warning;
#[path = "media_relay_logging/tests/fal_passthrough.rs"]
mod fal_passthrough;
#[path = "media_relay_logging/tests/fal_routes.rs"]
mod fal_routes;
#[path = "media_relay_logging/tests/listen_services.rs"]
mod listen_services;
#[path = "media_relay_logging/tests/support.rs"]
mod support;
