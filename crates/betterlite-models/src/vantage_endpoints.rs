use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::decisions::Json;
mod tests;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(try_from = "VantageInitRequestRaw")]
pub struct VantageInitRequest {
    pub api_key: String,
    pub integration_token: String,
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
    pub start_time_utc: Option<DateTime<Utc>>,
    pub end_time_utc: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, default)]
pub struct VantageDryRunRequest {
    pub limit: i64,
}

impl Default for VantageDryRunRequest {
    fn default() -> Self {
        let limit = 500;
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

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(try_from = "VantageSettingsUpdateRaw")]
pub struct VantageSettingsUpdate {
    pub api_key: Option<String>,
    pub integration_token: Option<String>,
    pub base_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct VantageInitRequestRaw {
    api_key: String,
    integration_token: String,
    #[serde(default = "default_base_url")]
    base_url: String,
}

impl TryFrom<VantageInitRequestRaw> for VantageInitRequest {
    type Error = String;

    fn try_from(raw: VantageInitRequestRaw) -> Result<Self, Self::Error> {
        convert_init_request(raw)
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct VantageSettingsUpdateRaw {
    api_key: Option<String>,
    integration_token: Option<String>,
    base_url: Option<String>,
}

impl TryFrom<VantageSettingsUpdateRaw> for VantageSettingsUpdate {
    type Error = String;

    fn try_from(raw: VantageSettingsUpdateRaw) -> Result<Self, Self::Error> {
        convert_settings_update(raw)
    }
}

fn default_base_url() -> String {
    let mut base_url = String::new();
    base_url.push_str("https://api.vantage.sh");
    base_url
}

fn validate_required_credentials(api_key: &str, integration_token: &str) -> Result<(), String> {
    if api_key.trim().is_empty() {
        return Err(String::from("api_key must be a non-empty string"));
    }
    if integration_token.trim().is_empty() {
        return Err(String::from("integration_token must be a non-empty string"));
    }
    Ok(())
}

fn convert_init_request(raw: VantageInitRequestRaw) -> Result<VantageInitRequest, String> {
    validate_required_credentials(&raw.api_key, &raw.integration_token)?;
    let request = VantageInitRequest {
        api_key: raw.api_key,
        integration_token: raw.integration_token,
        base_url: raw.base_url,
    };
    Ok(request)
}

fn validate_optional_credentials(
    api_key: Option<&str>,
    integration_token: Option<&str>,
) -> Result<(), String> {
    if api_key.is_some_and(str_trim_is_empty) || integration_token.is_some_and(str_trim_is_empty) {
        return Err(String::from("credentials must be non-empty strings"));
    }
    Ok(())
}

fn convert_settings_update(raw: VantageSettingsUpdateRaw) -> Result<VantageSettingsUpdate, String> {
    validate_optional_credentials(raw.api_key.as_deref(), raw.integration_token.as_deref())?;
    let update = VantageSettingsUpdate {
        api_key: raw.api_key,
        integration_token: raw.integration_token,
        base_url: raw.base_url,
    };
    Ok(update)
}

fn str_trim_is_empty(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.is_empty()
}
