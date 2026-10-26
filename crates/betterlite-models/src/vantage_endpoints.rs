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

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct VantageDryRunRequest {
    pub limit: Option<i64>,
}

impl<'de> Deserialize<'de> for VantageDryRunRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawDryRunRequest {
            #[serde(default)]
            limit: DryRunLimit,
        }

        let raw = RawDryRunRequest::deserialize(deserializer)?;
        Ok(Self {
            limit: match raw.limit {
                DryRunLimit::Missing => Some(500),
                DryRunLimit::Null => None,
                DryRunLimit::Number(limit) => Some(limit),
            },
        })
    }
}

#[derive(Default, Deserialize)]
#[serde(untagged)]
enum DryRunLimit {
    Null,
    Number(i64),
    #[default]
    Missing,
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
        if raw.api_key.trim().is_empty() {
            return Err(String::from("api_key must be a non-empty string"));
        }
        if raw.integration_token.trim().is_empty() {
            return Err(String::from("integration_token must be a non-empty string"));
        }
        Ok(Self {
            api_key: raw.api_key,
            integration_token: raw.integration_token,
            base_url: raw.base_url,
        })
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
        if raw.api_key.as_deref().is_some_and(str_trim_is_empty)
            || raw
                .integration_token
                .as_deref()
                .is_some_and(str_trim_is_empty)
        {
            return Err(String::from("credentials must be non-empty strings"));
        }
        Ok(Self {
            api_key: raw.api_key,
            integration_token: raw.integration_token,
            base_url: raw.base_url,
        })
    }
}

fn default_base_url() -> String {
    String::from("https://api.vantage.sh")
}

fn str_trim_is_empty(value: &str) -> bool {
    value.trim().is_empty()
}
