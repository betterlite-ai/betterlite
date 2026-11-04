use indexmap::IndexMap;

use betterlite_models::decisions::Json;
use betterlite_redact::error::RedactionError;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallbackMetadata {
    pub fields: IndexMap<String, Json>,
    pub skipped_fields: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum CallbackMetadataError {
    #[error("callback param '{field}' contains an os.environ/ reference")]
    EnvironmentReference { field: String },
    #[error("request metadata cannot set trusted internal field '{field}'")]
    TrustedField { field: String },
    #[error("credential redaction could not initialize: {source}")]
    Redactor {
        #[source]
        source: RedactionError,
    },
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

pub fn callback_metadata(metadata: &Json) -> Result<CallbackMetadata, CallbackMetadataError> {
    let mut fields = IndexMap::new();
    let mut skipped_fields = Vec::new();
    let Json::Object(input) = metadata else {
        return Ok(CallbackMetadata {
            fields,
            skipped_fields,
        });
    };
    for (field, field_value) in input {
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
        } else if contains_environment_reference(field_value) {
            return Err(CallbackMetadataError::EnvironmentReference {
                field: field.clone(),
            });
        } else {
            fields.insert(field.clone(), field_value.clone());
        }
    }
    Ok(CallbackMetadata {
        fields,
        skipped_fields,
    })
}

fn contains_environment_reference(value: &Json) -> bool {
    match value {
        Json::Text(text) => text.contains("os.environ/"),
        Json::List(values) => values.iter().any(contains_environment_reference),
        Json::Object(values) => values.values().any(contains_environment_reference),
        Json::Null | Json::Bool(_) | Json::Number(_) => false,
    }
}

#[must_use]
pub fn should_redact_message_logging(
    metadata: Option<&Json>,
    headers: Option<&IndexMap<String, Json>>,
    dynamic: Option<&Json>,
    global: bool,
) -> bool {
    let request_headers = headers.or_else(|| {
        let data = metadata?;
        object_field(data, "headers")
    });
    if request_headers.is_some_and(|header_fields| {
        header_fields
            .get("litellm-disable-message-redaction")
            .and_then(boolean_value)
            .unwrap_or(false)
    }) {
        return false;
    }
    if dynamic.and_then(boolean_value).is_some_and(|value| value) {
        return true;
    }
    if dynamic.and_then(boolean_value).is_some_and(|value| !value) {
        return false;
    }
    request_headers.is_some_and(|header_fields| {
        header_fields
            .get("litellm-enable-message-redaction")
            .and_then(boolean_value)
            .unwrap_or(false)
            || header_fields
                .get("x-litellm-enable-message-redaction")
                .and_then(boolean_value)
                .unwrap_or(false)
    }) || global
}

fn object_field<'source>(
    source: &'source Json,
    name: &str,
) -> Option<&'source IndexMap<String, Json>> {
    match source {
        Json::Object(fields) => match fields.get(name) {
            Some(Json::Object(object)) => Some(object),
            _ => None,
        },
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

fn boolean_value(json: &Json) -> Option<bool> {
    match json {
        Json::Bool(flag) => Some(*flag),
        Json::Text(text) => betterlite_env::flag("turn_off_message_logging", text)
            .ok()
            .flatten(),
        Json::Null | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

pub fn redacted_snapshot(value: &Json) -> Result<Json, CallbackMetadataError> {
    let mut snapshot = value.clone();
    redact_tree(&mut snapshot);
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
        .map_err(|source| CallbackMetadataError::Redactor { source })?;
    redact_credentials(&mut snapshot, None, &redactor);
    Ok(snapshot)
}

fn redact_tree(value: &mut Json) {
    match value {
        Json::Object(fields) => {
            for (key, child) in fields.iter_mut() {
                match key.as_str() {
                    "messages" => {
                        *child = Json::List(vec![Json::Object(IndexMap::from([
                            ("role".to_owned(), Json::Text("user".to_owned())),
                            ("content".to_owned(), Json::Text(REDACTED.to_owned())),
                        ]))]);
                    }
                    "prompt" | "input" => *child = Json::Text(String::new()),
                    "content" | "reasoning_content" | "text" | "refusal" | "arguments"
                        if !matches!(child, Json::Null) =>
                    {
                        *child = Json::Text(REDACTED.to_owned());
                    }
                    "content" | "reasoning_content" | "text" | "refusal" | "arguments" => {}
                    "thinking_blocks" | "audio" => *child = Json::Null,
                    _ => redact_tree(child),
                }
            }
        }
        Json::List(items) => items.iter_mut().for_each(redact_tree),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => {}
    }
}

fn redact_credentials(value: &mut Json, key: Option<&str>, redactor: &Redactor) {
    match value {
        Json::Object(fields) => {
            for (field, child) in fields {
                redact_credentials(child, Some(field), redactor);
            }
        }
        Json::List(values) => values
            .iter_mut()
            .for_each(|item| redact_credentials(item, key, redactor)),
        Json::Text(text) => *text = redactor.redact_structured_value(key, text),
        Json::Null | Json::Bool(_) | Json::Number(_) => {}
    }
}

#[must_use]
pub fn redact_user_api_key_info(metadata: &Json, enabled: bool) -> Json {
    if !enabled {
        return metadata.clone();
    }
    match metadata {
        Json::Object(fields) => Json::Object(
            fields
                .iter()
                .filter(|(key, _)| !key.starts_with("user_api_key"))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        ),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            metadata.clone()
        }
    }
}

#[path = "logging_privacy/tests.rs"]
mod tests;
