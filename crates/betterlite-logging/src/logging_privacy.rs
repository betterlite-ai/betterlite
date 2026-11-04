use serde_json::{Map, Value};

const REDACTED: &str = "[REDACTED]";
const BLOCKED_EXPORT_FIELDS: [&str; 10] = [
    "gcs_bucket_name",
    "gcs_path_service_account",
    "dd_api_key",
    "dd_site",
    "dd_agent_host",
    "dd_agent_port",
    "newrelic_api_key",
    "newrelic_region",
    "signoz_ingestion_endpoint",
    "signoz_ingestion_key",
];

#[derive(Debug, Clone, PartialEq)]
pub struct CallbackMetadata {
    pub fields: Map<String, Value>,
    pub skipped_fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CallbackMetadataError {
    #[error("callback param '{field}' contains an os.environ/ reference")]
    EnvironmentReference { field: String },
    #[error("request metadata cannot set trusted internal field '{field}'")]
    TrustedField { field: String },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PrivacyContext {
    pub message_logging_disabled: bool,
}

impl PrivacyContext {
    #[must_use]
    pub const fn child(self, requested_disabled: bool) -> Self {
        Self {
            message_logging_disabled: self.message_logging_disabled || requested_disabled,
        }
    }
}

pub fn callback_metadata(value: &Value) -> Result<CallbackMetadata, CallbackMetadataError> {
    let mut fields = Map::new();
    let mut skipped_fields = Vec::new();
    let Some(input) = value.as_object() else {
        return Ok(CallbackMetadata {
            fields,
            skipped_fields,
        });
    };
    for (field, value) in input {
        if matches!(
            field.as_str(),
            "trusted_callback_vars" | "internal_call_metadata" | "is_internal_call"
        ) {
            return Err(CallbackMetadataError::TrustedField {
                field: field.clone(),
            });
        }
        if BLOCKED_EXPORT_FIELDS.contains(&field.as_str()) {
            tracing::warn!(field = %field, "request-supplied callback field ignored for export");
            skipped_fields.push(field.clone());
            continue;
        }
        if contains_environment_reference(value) {
            return Err(CallbackMetadataError::EnvironmentReference {
                field: field.clone(),
            });
        }
        fields.insert(field.clone(), value.clone());
    }
    Ok(CallbackMetadata {
        fields,
        skipped_fields,
    })
}

fn contains_environment_reference(value: &Value) -> bool {
    match value {
        Value::String(text) => text.contains("os.environ/"),
        Value::Array(values) => values.iter().any(contains_environment_reference),
        Value::Object(values) => values.values().any(contains_environment_reference),
        Value::Null | Value::Bool(_) | Value::Number(_) => false,
    }
}

#[must_use]
pub fn should_redact_message_logging(
    metadata: Option<&Value>,
    headers: Option<&Map<String, Value>>,
    dynamic: Option<&Value>,
    global: bool,
) -> bool {
    if let Some(headers) = headers {
        if headers
            .get("litellm-disable-message-redaction")
            .and_then(as_boolish)
            .unwrap_or(false)
        {
            return false;
        }
        if headers
            .get("litellm-enable-message-redaction")
            .and_then(as_boolish)
            .unwrap_or(false)
            || headers
                .get("x-litellm-enable-message-redaction")
                .and_then(as_boolish)
                .unwrap_or(false)
        {
            return true;
        }
    }
    if let Some(value) = dynamic.and_then(as_boolish) {
        return value;
    }
    let _ = metadata;
    global
}

fn as_boolish(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(value) => Some(*value),
        Value::String(value) => betterlite_env::flag("turn_off_message_logging", value)
            .ok()
            .flatten(),
        _ => None,
    }
}

#[must_use]
pub fn redacted_snapshot(value: &Value) -> Value {
    let mut snapshot = value.clone();
    redact_tree(&mut snapshot);
    betterlite_redact::structured::mask_credentials_in_payload(&snapshot)
}

fn redact_tree(value: &mut Value) {
    match value {
        Value::Object(fields) => {
            for (key, child) in fields.iter_mut() {
                match key.as_str() {
                    "messages" => {
                        *child = serde_json::json!([{"role":"user","content":REDACTED}]);
                    }
                    "prompt" | "input" => *child = Value::String(String::new()),
                    "content" | "reasoning_content" | "text" | "refusal" | "arguments" => {
                        if !child.is_null() {
                            *child = Value::String(REDACTED.to_owned());
                        }
                    }
                    "thinking_blocks" | "audio" => *child = Value::Null,
                    "output" => redact_output(child),
                    _ => redact_tree(child),
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(redact_tree),
        _ => {}
    }
}

fn redact_output(value: &mut Value) {
    if let Some(items) = value.as_array_mut() {
        for item in items {
            if item.get("type").and_then(Value::as_str) == Some("reasoning") {
                if let Some(summary) = item.get_mut("summary").and_then(Value::as_array_mut) {
                    for entry in summary {
                        if entry.get("text").is_some_and(|text| !text.is_null()) {
                            entry["text"] = Value::String(REDACTED.to_owned());
                        }
                    }
                }
            }
            redact_tree(item);
        }
    }
}

#[must_use]
pub fn redact_user_api_key_info(metadata: &Value, enabled: bool) -> Value {
    if !enabled {
        return metadata.clone();
    }
    let Some(fields) = metadata.as_object() else {
        return metadata.clone();
    };
    Value::Object(
        fields
            .iter()
            .filter(|(key, _)| !key.starts_with("user_api_key"))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
    )
}

#[path = "logging_privacy/tests.rs"]
mod tests;
