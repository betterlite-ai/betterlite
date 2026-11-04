pub mod callbacks;
mod credentials;
pub mod internal;
pub mod request;
pub mod snapshot;

use indexmap::IndexMap;

use betterlite_models::decisions::Json;
use betterlite_redact::error::RedactionError;
use betterlite_redact::secret::Redactor;

const REDACTED: &str = "redacted-by-litellm";
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

#[derive(Clone, PartialEq, Eq)]
pub struct CallbackMetadata {
    pub fields: IndexMap<String, Json>,
    pub skipped_fields: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum CallbackMetadataError {
    #[error("invalid callback field {field}")]
    InvalidField { field: String },
    #[error("invalid privacy metadata: {source}")]
    Json {
        #[from]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Flag {
        #[from]
        source: betterlite_env::EnvError,
    },
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
        if !callbacks::SUPPORTED.contains(&field.as_str())
            && !BLOCKED_EXPORT_FIELDS.contains(&field.as_str())
            && !callbacks::TRUSTED_FIELDS.contains(&field.as_str())
        {
            continue;
        }
        validate_request_field(field, field_value)?;
        if BLOCKED_EXPORT_FIELDS.contains(&field.as_str())
            || callbacks::DESTINATIONS.contains(&field.as_str())
        {
            tracing::warn!(field = %field, "request-supplied callback field ignored for export");
            skipped_fields.push(field.clone());
        } else {
            fields.insert(field.clone(), field_value.clone());
        }
    }
    Ok(CallbackMetadata {
        fields,
        skipped_fields,
    })
}

fn validate_request_field(field: &str, value: &Json) -> Result<(), CallbackMetadataError> {
    if callbacks::TRUSTED_FIELDS.contains(&field) {
        return Err(CallbackMetadataError::TrustedField {
            field: field.to_owned(),
        });
    }
    if BLOCKED_EXPORT_FIELDS.contains(&field) || callbacks::DESTINATIONS.contains(&field) {
        return Ok(());
    }
    if contains_environment_reference(value) {
        return Err(CallbackMetadataError::EnvironmentReference {
            field: field.to_owned(),
        });
    }
    if field == "langfuse_environment" {
        let Json::Text(environment) = value else {
            return Err(CallbackMetadataError::InvalidField {
                field: field.to_owned(),
            });
        };
        callbacks::validate_environment(environment)?;
    }
    Ok(())
}

fn contains_environment_reference(value: &Json) -> bool {
    match value {
        Json::Text(text) => text.contains("os.environ/"),
        Json::List(values) => values.iter().any(contains_environment_reference),
        Json::Object(values) => values.values().any(contains_environment_reference),
        Json::Null | Json::Bool(_) | Json::Number(_) => false,
    }
}

pub fn callback_snapshot(
    mut value: Json,
    decision: request::Decision,
    redactor: &Redactor,
) -> Json {
    if decision.content {
        redact_tree(&mut value);
    }
    credentials::scrub(&mut value, redactor);
    value
}

fn redact_tree(value: &mut Json) {
    match value {
        Json::Object(fields) => {
            scrub_provider_metadata(fields);
            for (key, child) in fields.iter_mut() {
                match key.as_str() {
                    "messages" => {
                        *child = Json::List(vec![Json::Object(IndexMap::from([
                            ("role".to_owned(), Json::Text("user".to_owned())),
                            ("content".to_owned(), Json::Text(REDACTED.to_owned())),
                        ]))])
                    }
                    "prompt" | "input" => *child = Json::Text(String::new()),
                    "content" | "reasoning_content" | "text" | "refusal" | "arguments"
                        if !matches!(child, Json::Null) =>
                    {
                        *child = Json::Text(REDACTED.to_owned())
                    }
                    "thinking_blocks" | "audio" | "logprobs" | "annotations" => *child = Json::Null,
                    _ => redact_tree(child),
                }
            }
        }
        Json::List(items) => items.iter_mut().for_each(redact_tree),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => {}
    }
}

fn scrub_provider_metadata(fields: &mut IndexMap<String, Json>) {
    if fields.get("object") == Some(&Json::Text("list".to_owned())) && fields.contains_key("data") {
        fields.insert("data".to_owned(), Json::List(Vec::new()));
    }
    for field in [
        "served_output_texts",
        "classifier_input",
        "originating_request_masked",
    ] {
        fields.shift_remove(field);
    }
    for field in [
        "vertex_ai_grounding_metadata",
        "vertex_ai_url_context_metadata",
        "vertex_ai_safety_ratings",
        "vertex_ai_safety_results",
        "vertex_ai_citation_metadata",
    ] {
        if let Some(value) = fields.get_mut(field) {
            *value = Json::List(Vec::new());
        }
        for hidden in ["_hidden_params", "hidden_params"] {
            if let Some(Json::Object(params)) = fields.get_mut(hidden) {
                params.shift_remove(field);
            }
        }
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

impl std::fmt::Debug for CallbackMetadata {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CallbackMetadata")
            .field("fields", &self.fields.keys().collect::<Vec<_>>())
            .field("skipped_fields", &self.skipped_fields)
            .finish()
    }
}
