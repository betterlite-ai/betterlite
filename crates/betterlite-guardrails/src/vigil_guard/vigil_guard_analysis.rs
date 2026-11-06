use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use indexmap::IndexMap;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};

use super::ANALYZE_ENDPOINT;
use super::vigil_guard_config::VigilGuardSettings;
use super::vigil_guard_error::VigilGuardError;
use super::vigil_guard_handler_protocol::AsyncPostHandler;
use super::vigil_guard_metadata::MetadataValue;
use super::vigil_guard_retry::post_with_retry;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    UserInput,
    ModelOutput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum AnalyzeMode {
    #[serde(rename = "full")]
    Full,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AnalyzePayload {
    pub text: String,
    pub source: Source,
    pub mode: AnalyzeMode,
    pub metadata: IndexMap<String, MetadataValue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Decision {
    #[serde(rename = "ALLOWED")]
    Allowed,
    #[serde(rename = "SANITIZED")]
    Sanitized,
    #[serde(rename = "BLOCKED")]
    Blocked,
}

impl Decision {
    #[must_use]
    pub fn from_value(value: &GuardrailValue) -> Option<Self> {
        match value {
            GuardrailValue::Text(name) => match name.as_str() {
                "ALLOWED" => Some(Self::Allowed),
                "SANITIZED" => Some(Self::Sanitized),
                "BLOCKED" => Some(Self::Blocked),
                _ => None,
            },
            GuardrailValue::Null
            | GuardrailValue::Boolean(_)
            | GuardrailValue::SignedInteger(_)
            | GuardrailValue::UnsignedInteger(_)
            | GuardrailValue::Decimal(_)
            | GuardrailValue::Array(_)
            | GuardrailValue::Object(_) => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Analysis {
    pub decision: Option<GuardrailValue>,
    pub block_message: Option<GuardrailValue>,
    pub decision_reason: Option<GuardrailValue>,
    pub categories: Option<GuardrailValue>,
    pub sanitized_text: Option<GuardrailValue>,
    pub output_text: Option<GuardrailValue>,
}

pub async fn analyze<H>(
    handler: &H,
    settings: &VigilGuardSettings,
    text: &str,
    source: Source,
    metadata: &IndexMap<String, MetadataValue>,
) -> Result<Analysis, VigilGuardError>
where
    H: AsyncPostHandler,
{
    let payload = AnalyzePayload {
        text: text.to_owned(),
        source,
        mode: AnalyzeMode::Full,
        metadata: metadata.clone(),
    };
    let endpoint = format!("{}{}", settings.api_base, ANALYZE_ENDPOINT);
    let headers = request_headers(&settings.api_key)?;
    let response =
        post_with_retry(handler, &endpoint, &headers, &payload, settings.timeout).await?;
    serde_json::from_slice(&response.body)
        .map_err(|error| VigilGuardError::ResponseJson { source: error })
}

fn request_headers(api_key: &SecretString) -> Result<HeaderMap, VigilGuardError> {
    let key = api_key.expose_secret();
    let bearer = format!("Bearer {key}");
    let mut authorization =
        HeaderValue::from_str(&bearer).map_err(|source| VigilGuardError::InvalidHeader {
            name: "Authorization",
            source,
        })?;
    authorization.set_sensitive(true);
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, authorization);
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    Ok(headers)
}
