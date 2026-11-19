use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::argilla_logging::logger::RandomSource;
use crate::logging_success::payload_state::STANDARD_LOGGING_OBJECT;
use crate::rubrik::request_data::apply_correlation_id;
use crate::rubrik::system_prompt::prepend_system_prompt;
use crate::rubrik_moderation::context::text_field;

use super::error::EventPayloadError;
use super::event::EventKind;

pub fn prepare_log_payload(
    random: &dyn RandomSource,
    sampling_rate: f64,
    call_details: &IndexMap<String, Json>,
    event: EventKind,
) -> Result<Option<IndexMap<String, Json>>, EventPayloadError> {
    // LiteLLM drops a draw only when it is above the rate, so a rate of 1.0 keeps every event.
    if random.sample() > sampling_rate {
        tracing::debug!(sampling_rate, event = %event, "Skipping Rubrik logging");
        return Ok(None);
    }
    let Some(standard_logging_object) = call_details.get(STANDARD_LOGGING_OBJECT) else {
        return Err(EventPayloadError::MissingStandardLoggingObject);
    };
    let Json::Object(base) = standard_logging_object else {
        return Err(EventPayloadError::FieldType {
            field: STANDARD_LOGGING_OBJECT,
        });
    };
    // Other callbacks share this object, so the copy comes before any mutation.
    let mut payload = base.clone();
    apply_correlation_id(&mut payload, text_field(call_details, "litellm_call_id")?);
    if let Some(messages) =
        prepend_system_prompt(payload.get("messages"), call_details.get("system"))
    {
        payload.insert("messages".to_owned(), Json::List(messages));
    }
    Ok(Some(payload))
}
