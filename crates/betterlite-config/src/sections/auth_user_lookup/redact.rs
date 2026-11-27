use betterlite_db::budget_permission_rows::JsonData;
use betterlite_redact::setting_redaction::redact_secret_values;
use betterlite_redact::structured::REDACTED;
use indexmap::IndexMap;

// proxy_server.py's debug line for litellm.default_internal_user_params walks the whole map from depth 0
#[must_use]
pub(super) fn redacted_params(values: &IndexMap<String, JsonData>) -> JsonData {
    let Ok(payload) = serde_json::to_value(values) else {
        return JsonData::String(REDACTED.to_owned());
    };
    let redacted = redact_secret_values(&payload);
    serde_json::from_value(redacted).unwrap_or_else(|_error| JsonData::String(REDACTED.to_owned()))
}
