use crate::fields::payload::StandardLoggingPayload;
use crate::logging_success::request_redaction::redact_payload;

// redact_messages.redacted_standard_logging_payload: a callback that batches failure payloads redacts them itself.
#[must_use]
pub fn redact_standard_payload(payload: &StandardLoggingPayload) -> StandardLoggingPayload {
    let mut redacted = payload.clone();
    redact_payload(&mut redacted);
    redacted
}
