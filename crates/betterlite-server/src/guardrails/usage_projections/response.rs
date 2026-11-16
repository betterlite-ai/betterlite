use std::collections::BTreeMap;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::status::{UsageStatus, UsageTrend};

// score stays absent until something fills it
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageChartPoint {
    pub date: String,
    pub passed: i64,
    pub blocked: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageOverviewRow {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub provider: String,
    #[serde(rename = "requestsEvaluated")]
    pub requests_evaluated: i64,
    #[serde(rename = "failRate")]
    pub fail_rate: f64,
    #[serde(rename = "avgScore")]
    pub avg_score: Option<f64>,
    #[serde(rename = "avgLatency")]
    pub avg_latency: Option<f64>,
    pub status: UsageStatus,
    pub trend: UsageTrend,
    #[serde(rename = "usageUnits")]
    pub usage_units: BTreeMap<String, i64>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub cost: Option<Decimal>,
    #[serde(rename = "untrackedUsageUnits")]
    pub untracked_usage_units: BTreeMap<String, i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageOverviewResponse {
    pub rows: Vec<UsageOverviewRow>,
    pub chart: Vec<UsageChartPoint>,
    #[serde(rename = "totalRequests")]
    pub total_requests: i64,
    #[serde(rename = "totalBlocked")]
    pub total_blocked: i64,
    #[serde(rename = "passRate")]
    pub pass_rate: f64,
    #[serde(rename = "totalUsageUnits")]
    pub total_usage_units: BTreeMap<String, i64>,
    #[serde(
        rename = "totalCost",
        with = "rust_decimal::serde::arbitrary_precision_option"
    )]
    pub total_cost: Option<Decimal>,
    #[serde(rename = "totalUntrackedUsageUnits")]
    pub total_untracked_usage_units: BTreeMap<String, i64>,
}

// what LiteLLM answers with before any row or price exists
pub static EMPTY_OVERVIEW: UsageOverviewResponse = UsageOverviewResponse {
    rows: Vec::new(),
    chart: Vec::new(),
    total_requests: 0,
    total_blocked: 0,
    pass_rate: 100.0,
    total_usage_units: BTreeMap::new(),
    total_cost: None,
    total_untracked_usage_units: BTreeMap::new(),
};

mod tests;
