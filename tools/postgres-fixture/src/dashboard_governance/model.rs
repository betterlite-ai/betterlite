use rust_decimal::Decimal;
use serde::Deserialize;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputPolicy {
    Trusted,
    #[default]
    Untrusted,
    Blocked,
}

impl InputPolicy {
    #[must_use]
    pub const fn as_wire(self) -> &'static str {
        match self {
            Self::Trusted => "trusted",
            Self::Untrusted => "untrusted",
            Self::Blocked => "blocked",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputPolicy {
    Trusted,
    #[default]
    Untrusted,
}

impl OutputPolicy {
    #[must_use]
    pub const fn as_wire(self) -> &'static str {
        match self {
            Self::Trusted => "trusted",
            Self::Untrusted => "untrusted",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuardrailMetricRow {
    pub guardrail_id: String,
    pub date: String,
    #[serde(default)]
    pub requests_evaluated: i64,
    #[serde(default)]
    pub passed_count: i64,
    #[serde(default)]
    pub blocked_count: i64,
    #[serde(default)]
    pub flagged_count: i64,
    #[serde(default)]
    pub avg_score: Option<Decimal>,
    #[serde(default)]
    pub avg_latency_ms: Option<Decimal>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageUnitRow {
    pub guardrail_id: String,
    pub date: String,
    #[serde(default)]
    pub team_id: String,
    #[serde(default)]
    pub key_index: Option<usize>,
    pub usage_unit: String,
    #[serde(default)]
    pub units: i64,
    #[serde(default)]
    pub cost: Option<Decimal>,
    #[serde(default)]
    pub untracked_units: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuardrailIndexRow {
    pub request_id: String,
    pub guardrail_id: String,
    #[serde(default)]
    pub policy_id: Option<String>,
    pub start_time: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolIndexRow {
    pub request_id: String,
    pub tool_name: String,
    pub start_time: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolRow {
    pub tool_id: String,
    pub tool_name: String,
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(default)]
    pub input_policy: InputPolicy,
    #[serde(default)]
    pub output_policy: OutputPolicy,
    #[serde(default)]
    pub call_count: i32,
    #[serde(default)]
    pub key_index: Option<usize>,
    #[serde(default)]
    pub team_id: Option<String>,
    #[serde(default)]
    pub key_alias: Option<String>,
    #[serde(default)]
    pub user_agent: Option<String>,
    #[serde(default)]
    pub last_used_at: Option<String>,
}
