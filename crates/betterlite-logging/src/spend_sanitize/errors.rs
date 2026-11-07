use betterlite_models::decisions::Json;

use super::SpendSanitizePolicy;
use super::prompt_leaks::redact_spend_error_text;
use super::request_body::sanitize_request_body;
use super::value::{SpendMapping, SpendValue};

const LEAK_FIELDS: [&str; 2] = ["error_message", "traceback"];

#[must_use]
pub fn sanitize_error_information(
    information: &SpendMapping,
    policy: &SpendSanitizePolicy,
    safe_spend_message: Option<&str>,
) -> Json {
    let mut persisted = information.clone();
    if let Some(message) = safe_spend_message {
        persisted.entries.insert(
            "error_message".to_owned(),
            SpendValue::Text(message.to_owned()),
        );
    }
    if !policy.store_bodies {
        for field in LEAK_FIELDS {
            if let Some(SpendValue::Text(text)) = persisted.entries.get_mut(field) {
                *text = redact_spend_error_text(text);
            }
        }
    }
    sanitize_request_body(&persisted, policy.max_string_length)
}
