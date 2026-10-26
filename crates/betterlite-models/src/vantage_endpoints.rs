use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};

use crate::decisions::Json;
mod tests;

#[derive(Clone, Debug, Deserialize)]
#[serde(try_from = "VantageInitRequestRaw")]
pub struct VantageInitRequest {
    pub api_key: SecretString,
    pub integration_token: SecretString,
    #[serde(default = "default_base_url")]
    pub base_url: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct VantageInitResponse {
    pub message: String,
    pub status: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct VantageExportRequest {
    pub limit: Option<i64>,
    #[serde(
        default,
        deserialize_with = "crate::finops_datetime::deserialize_optional_datetime"
    )]
    pub start_time_utc: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(
        default,
        deserialize_with = "crate::finops_datetime::deserialize_optional_datetime"
    )]
    pub end_time_utc: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, default)]
pub struct VantageDryRunRequest {
    pub limit: Option<i64>,
}

impl Default for VantageDryRunRequest {
    fn default() -> Self {
        let limit = Some(500);
        Self { limit }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct VantageExportResponse {
    pub message: String,
    pub status: String,
    pub dry_run_data: Option<Json>,
    pub summary: Option<Json>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct VantageSettingsView {
    pub api_key_masked: Option<String>,
    pub integration_token_masked: Option<String>,
    pub base_url: Option<String>,
    pub status: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(try_from = "VantageSettingsUpdateRaw")]
pub struct VantageSettingsUpdate {
    pub api_key: Option<SecretString>,
    pub integration_token: Option<SecretString>,
    pub base_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct VantageInitRequestRaw {
    api_key: SecretString,
    integration_token: SecretString,
    #[serde(default = "default_base_url")]
    base_url: String,
}

impl TryFrom<VantageInitRequestRaw> for VantageInitRequest {
    type Error = VantageValidationError;

    fn try_from(raw: VantageInitRequestRaw) -> Result<Self, Self::Error> {
        let VantageInitRequestRaw {
            api_key,
            integration_token,
            base_url,
        } = raw;
        match (
            api_key.expose_secret().trim().is_empty(),
            integration_token.expose_secret().trim().is_empty(),
        ) {
            (true, _) => Err(VantageValidationError::EmptyApiKey),
            (false, true) => Err(VantageValidationError::EmptyIntegrationToken),
            (false, false) => Ok(Self {
                api_key,
                integration_token,
                base_url,
            }),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct VantageSettingsUpdateRaw {
    api_key: Option<SecretString>,
    integration_token: Option<SecretString>,
    base_url: Option<String>,
}

impl TryFrom<VantageSettingsUpdateRaw> for VantageSettingsUpdate {
    type Error = VantageValidationError;

    fn try_from(raw: VantageSettingsUpdateRaw) -> Result<Self, Self::Error> {
        validate_credentials(raw.api_key.as_ref(), raw.integration_token.as_ref())?;
        let (api_key, integration_token, base_url) =
            (raw.api_key, raw.integration_token, raw.base_url);
        Ok(Self {
            api_key,
            integration_token,
            base_url,
        })
    }
}

fn default_base_url() -> String {
    let mut base_url = String::new();
    base_url.push_str("https://api.vantage.sh");
    base_url
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum VantageValidationError {
    #[error("api_key must be a non-empty string")]
    EmptyApiKey,
    #[error("integration_token must be a non-empty string")]
    EmptyIntegrationToken,
}

fn validate_credentials(
    api_key: Option<&SecretString>,
    integration_token: Option<&SecretString>,
) -> Result<(), VantageValidationError> {
    if api_key.is_some_and(|value| value.expose_secret().trim().is_empty()) {
        return Err(VantageValidationError::EmptyApiKey);
    }
    if integration_token.is_some_and(|value| value.expose_secret().trim().is_empty()) {
        return Err(VantageValidationError::EmptyIntegrationToken);
    }
    Ok(())
}
