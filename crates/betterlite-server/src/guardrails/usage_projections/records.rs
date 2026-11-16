use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use rust_decimal::Decimal;
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct UsageCounterRow {
    pub usage_unit: String,
    pub guardrail_id: String,
    pub team_id: String,
    #[serde(rename = "api_key")]
    pub hashed_api_key: String,
    pub units: i64,
    pub untracked_units: i64,
    #[serde(default)]
    pub cost: Option<Decimal>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct UsageMetricRow {
    pub date: String,
    pub id: String,
    #[serde(default)]
    pub requests_evaluated: Option<i64>,
    #[serde(default)]
    pub passed_count: Option<i64>,
    #[serde(default)]
    pub blocked_count: Option<i64>,
    #[serde(default)]
    pub flagged_count: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PolicyRecord {
    pub policy_id: String,
    #[serde(default)]
    pub policy_name: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct UsageGuardrailParams {
    #[serde(default)]
    pub guardrail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct GuardrailRecord {
    #[serde(default)]
    pub guardrail_id: Option<String>,
    #[serde(default)]
    pub guardrail_name: Option<String>,
    #[serde(default)]
    pub litellm_params: UsageGuardrailParams,
    #[serde(default)]
    pub guardrail_info: Option<OpaqueParams>,
}

mod tests;
