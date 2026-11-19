use std::collections::BTreeMap;

use betterlite_config::sections::azure_search::{MODEL_ENV, PROJECT_ENDPOINT_ENV, TOKEN_ENV};
use betterlite_openai::search::credentials::CredentialError;
use serde::Deserialize;
use thiserror::Error;

use super::{DOCS_URL, ENTRA_SCOPE};

#[derive(Debug, Clone, Deserialize)]
pub struct ErrorBody {
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ErrorEnvelope {
    #[serde(default)]
    pub(crate) error: Option<ErrorBody>,
}

#[derive(Debug, Error)]
pub enum AzureSearchError {
    #[error(
        "{} is not set. Set it to your Microsoft Foundry project endpoint, e.g. https://<account>.services.ai.azure.com/api/projects/<project>.",
        PROJECT_ENDPOINT_ENV
    )]
    MissingProjectEndpoint,
    #[error(
        "{} is not set. Set it to a model deployment in the Foundry project that runs the grounded search, e.g. gpt-4.1.",
        MODEL_ENV
    )]
    MissingModel,
    #[error(
        "Grounding with Bing Search: no credential available. Pass api_key, set {} to an Entra bearer token, or configure azure-identity (AZURE_CLIENT_ID / AZURE_CLIENT_SECRET / AZURE_TENANT_ID or any DefaultAzureCredential source) for scope {}. Underlying error: {detail}",
        TOKEN_ENV,
        ENTRA_SCOPE
    )]
    NoCredential { detail: String },
    #[error(transparent)]
    Credential(#[from] CredentialError),
    #[error("Grounding with Bing Search: {detail}. See {} for details.", DOCS_URL)]
    Provider {
        status_code: u16,
        headers: BTreeMap<String, String>,
        detail: String,
    },
}

impl AzureSearchError {
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::Provider { status_code, .. } => Some(*status_code),
            Self::MissingProjectEndpoint
            | Self::MissingModel
            | Self::NoCredential { .. }
            | Self::Credential(_) => None,
        }
    }
}

// Foundry nests a second JSON document as a string inside error.message, so the unwrap runs twice
#[must_use]
pub fn unwrap_error_detail(message: &str) -> String {
    let Ok(envelope) = serde_json::from_str::<ErrorEnvelope>(message) else {
        return message.to_owned();
    };
    let Some(nested_text) = envelope.error.and_then(|body| body.message) else {
        return message.to_owned();
    };
    match serde_json::from_str::<ErrorBody>(&nested_text) {
        // an empty inner message keeps the whole nested document, as LiteLLM's `or` does
        Ok(body) => body
            .message
            .filter(|inner| !inner.is_empty())
            .unwrap_or(nested_text),
        Err(_) => nested_text,
    }
}

#[must_use]
pub fn provider_error(
    message: &str,
    status_code: u16,
    headers: &[(String, String)],
) -> AzureSearchError {
    let detail = unwrap_error_detail(message)
        .trim_end_matches(['.', ' '])
        .to_owned();
    AzureSearchError::Provider {
        status_code,
        headers: headers
            .iter()
            .map(|(name, value)| (name.to_ascii_lowercase(), value.clone()))
            .collect(),
        detail,
    }
}
