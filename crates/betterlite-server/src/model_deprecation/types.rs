use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

pub const DEFAULT_DEPRECATION_WARN_DAYS: i32 = 30;

// LiteLLM's DEFAULT_DEPRECATION_CHECK_INTERVAL_SECONDS; the longer Rust name breaks no-long-names
pub const DEFAULT_CHECK_INTERVAL_SECONDS: u64 = 86_400;

pub const DEPRECATION_IDLE_POLL_SECONDS: u64 = 30;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeprecationStatus {
    Deprecated,
    Imminent,
    Upcoming,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ModelDeprecationInfo {
    pub model_name: String,
    #[serde(default)]
    pub litellm_model: Option<String>,
    pub deprecation_date: NaiveDate,
    pub days_until_deprecation: i64,
    pub status: DeprecationStatus,
    #[serde(default)]
    pub litellm_provider: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ModelDeprecationResponse {
    #[serde(default)]
    pub deprecated: Vec<ModelDeprecationInfo>,
    #[serde(default)]
    pub imminent: Vec<ModelDeprecationInfo>,
    #[serde(default)]
    pub upcoming: Vec<ModelDeprecationInfo>,
    pub warn_within_days: i32,
    pub checked_at: DateTime<Utc>,
}
