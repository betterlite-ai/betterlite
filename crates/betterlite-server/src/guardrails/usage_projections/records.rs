use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use rust_decimal::Decimal;
use serde::Deserialize;

/// One row of the daily guardrail usage-unit table, as the dashboard projections read it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct UsageCounterRow {
    pub usage_unit: String,
    pub guardrail_id: String,
    pub team_id: String,
    pub api_key: String,
    pub units: i64,
    pub untracked_units: i64,
    #[serde(default)]
    pub cost: Option<Decimal>,
}

/// One daily guardrail or policy metrics row; `id` holds the table's own id column.
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

/// A policy record as the overview reads it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PolicyRecord {
    pub policy_id: String,
    #[serde(default)]
    pub policy_name: Option<String>,
}

/// The part of a guardrail's `litellm_params` that the overview reads.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct UsageGuardrailParams {
    #[serde(default)]
    pub guardrail: Option<String>,
}

/// A database row or config block, the two shapes LiteLLM's field helpers accept.
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
