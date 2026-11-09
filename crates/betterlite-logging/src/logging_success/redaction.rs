use super::payload_state::{payload_from_object, payload_to_object};
use super::types::SuccessError;
use crate::callback_logger::CallbackLogger;
use crate::exporter_options::redaction::{
    ModelCallDetails,
    Redaction,
    RedactionPolicy,
    ResponseStamp,
    // ast-grep-ignore: no-long-names
    redact_standard_logging_payload,
};
use crate::fields::payload::StandardLoggingPayload;

// Applies B3931's per-callback redaction to the typed payload the logger receives
pub(super) fn redacted_payload(
    payload: &StandardLoggingPayload,
    logger: &dyn CallbackLogger,
    excluded_fields: &[String],
    stamp: &ResponseStamp,
) -> Result<StandardLoggingPayload, SuccessError> {
    let turn_off_message_logging = logger.turn_off_message_logging();
    if !turn_off_message_logging && excluded_fields.is_empty() {
        return Ok(payload.clone());
    }
    let original = payload_to_object(payload)?;
    let details = ModelCallDetails {
        standard_logging_object: Some(original.clone()),
        proxy_server_request: None,
    };
    let policy = RedactionPolicy {
        turn_off_message_logging,
        excluded_fields: excluded_fields.to_vec(),
        callback_redacts_itself: logger.redacts_messages_itself(),
    };
    let Redaction::Redacted {
        standard_logging_object,
        ..
        // ast-grep-ignore: no-long-names
    } = redact_standard_logging_payload(&details, &policy, stamp)
    else {
        return Ok(payload.clone());
    };
    if let Ok(redacted) = payload_from_object(&standard_logging_object) {
        return Ok(redacted);
    }
    // The typed payload has required fields, so an excluded required field stays with a warning
    let excluded_required: Vec<String> = original
        .keys()
        .filter(|key| !standard_logging_object.contains_key(*key))
        .cloned()
        .collect();
    tracing::warn!(
        callback = logger.callback_name(),
        fields = ?excluded_required,
        "the typed standard logging payload cannot drop these excluded fields"
    );
    let mut merged = original;
    for (key, value) in standard_logging_object {
        merged.insert(key, value);
    }
    payload_from_object(&merged)
}
