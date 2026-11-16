use betterlite_config::sections::settings::JsonValue;

use crate::router_runtime_settings::error::RouterSettingsError;

// LiteLLM's RetryPolicy fields, the names pydantic writes them with
const POLICY_FIELDS: [&str; 9] = [
    "BadRequestErrorRetries",
    "AuthenticationErrorRetries",
    "TimeoutErrorRetries",
    "RateLimitErrorRetries",
    "ContentPolicyViolationErrorRetries",
    "InternalServerErrorRetries",
    "ServiceUnavailableErrorRetries",
    "NotFoundErrorRetries",
    "DefaultRetries",
];

// None keeps the stored policy: LiteLLM drops a value that is neither an object nor null
pub fn retry_policy_value(value: &JsonValue) -> Result<Option<JsonValue>, RouterSettingsError> {
    let fields = match value {
        JsonValue::Null => return Ok(Some(JsonValue::Null)),
        JsonValue::Object(fields) => fields,
        JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::String(_) | JsonValue::Array(_) => {
            return Ok(None);
        }
    };
    let mut entries: Vec<(String, JsonValue)> = Vec::with_capacity(POLICY_FIELDS.len());
    for field in POLICY_FIELDS {
        let stored = match fields.get(field) {
            Some(retries) => {
                serde_json::from_value::<Option<i64>>(retries.clone()).map_err(|source| {
                    RouterSettingsError::RetryPolicy {
                        field: field.to_owned(),
                        source,
                    }
                })?;
                retries.clone()
            }
            None => JsonValue::Null,
        };
        entries.push((field.to_owned(), stored));
    }
    Ok(Some(JsonValue::Object(entries.into_iter().collect())))
}
