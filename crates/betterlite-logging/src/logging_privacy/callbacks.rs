use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use super::CallbackMetadataError;
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
    pub newrelic_api_key: Option<SecretString>,
    pub newrelic_region: Option<String>,
    pub signoz_ingestion_key: Option<SecretString>,
    pub signoz_ingestion_endpoint: Option<String>,
}

impl TrustedCallbacks {
    pub fn overlay(&self, dynamic: &mut IndexMap<String, Json>) {
        for (name, value) in [
            (
                "newrelic_api_key",
                self.newrelic_api_key
                    .as_ref()
                    .map(|secret| secret.expose_secret()),
            ),
            ("newrelic_region", self.newrelic_region.as_deref()),
            (
                "signoz_ingestion_key",
                self.signoz_ingestion_key
                    .as_ref()
                    .map(|secret| secret.expose_secret()),
            ),
            (
                "signoz_ingestion_endpoint",
                self.signoz_ingestion_endpoint.as_deref(),
            ),
        ] {
            if let Some(text) = value {
                dynamic.insert(name.to_owned(), Json::Text(text.to_owned()));
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
