use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;

use super::{CallbackMetadata, CallbackMetadataError};
use betterlite_models::decisions::Json;

pub const TRUSTED_FIELDS: [&str; 5] = [
    "trusted_callback_vars",
    "litellm_trusted_callback_vars",
    "internal_call_metadata",
    "is_internal_call",
    "internal_call_origin",
];

pub const DESTINATIONS: [&str; 5] = [
    "langfuse_host",
    "langsmith_base_url",
    "posthog_host",
    "braintrust_host",
    "slack_webhook_url",
];

pub const SUPPORTED: [&str; 25] = [
    "langfuse_public_key",
    "langfuse_secret",
    "langfuse_secret_key",
    "langfuse_host",
    "langfuse_environment",
    "langfuse_prompt_version",
    "langsmith_api_key",
    "langsmith_project",
    "langsmith_base_url",
    "langsmith_sampling_rate",
    "langsmith_tenant_id",
    "humanloop_api_key",
    "arize_api_key",
    "arize_space_key",
    "arize_space_id",
    "arize_success_sampling_rate",
    "arize_error_sampling_rate",
    "posthog_api_key",
    "posthog_host",
    "braintrust_api_key",
    "braintrust_project",
    "braintrust_host",
    "slack_webhook_url",
    "lunary_public_key",
    "turn_off_message_logging",
];

// Configured credentials use a separate channel that request deserialization cannot construct.
#[derive(Default)]
pub struct TrustedCallbacks {
    pub values: IndexMap<String, SecretString>,
}

impl TrustedCallbacks {
    pub fn overlay(&self, dynamic: &mut IndexMap<String, SecretString>) {
        for (name, value) in &self.values {
            if matches!(
                name.as_str(),
                "newrelic_api_key"
                    | "newrelic_region"
                    | "signoz_ingestion_key"
                    | "signoz_ingestion_endpoint"
            ) {
                dynamic.insert(name.clone(), value.clone());
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpanScope {
    Full,
    LlmOnly,
}

pub fn validate_environment(value: &str) -> Result<(), CallbackMetadataError> {
    if value.is_empty()
        || value.starts_with("langfuse")
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        })
    {
        Err(CallbackMetadataError::InvalidField {
            field: "langfuse_environment".to_owned(),
        })
    } else {
        Ok(())
    }
}

mod tests;

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
        if !SUPPORTED.contains(&field.as_str())
            && !BLOCKED_EXPORT_FIELDS.contains(&field.as_str())
            && !TRUSTED_FIELDS.contains(&field.as_str())
        {
            continue;
        }
        validate_request_field(field, field_value)?;
        if BLOCKED_EXPORT_FIELDS.contains(&field.as_str()) || DESTINATIONS.contains(&field.as_str())
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
    if TRUSTED_FIELDS.contains(&field) {
        return Err(CallbackMetadataError::TrustedField {
            field: field.to_owned(),
        });
    }
    if BLOCKED_EXPORT_FIELDS.contains(&field) || DESTINATIONS.contains(&field) {
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
        validate_environment(environment)?;
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
