use betterlite_models::decisions::Json;
use chrono::NaiveDate;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

// money fields are Decimal here and serialize as LiteLLM's JSON floats through serde's float adapter
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SpendMetrics {
    #[serde(with = "rust_decimal::serde::float")]
    pub spend: Decimal,
    #[serde(with = "rust_decimal::serde::float")]
    pub flat_cost: Decimal,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub cache_read_input_tokens: i64,
    pub cache_creation_input_tokens: i64,
    pub compression_saved_tokens: i64,
    #[serde(with = "rust_decimal::serde::float")]
    pub compression_savings_spend: Decimal,
    #[serde(with = "rust_decimal::serde::float")]
    pub prompt_caching_savings_spend: Decimal,
    #[serde(
        rename = "gateway_injected_caching_savings_spend",
        with = "rust_decimal::serde::float"
    )]
    pub gateway_caching_savings: Decimal,
    #[serde(with = "rust_decimal::serde::float")]
    pub autorouter_savings_spend: Decimal,
    pub total_tokens: i64,
    pub successful_requests: i64,
    pub failed_requests: i64,
    pub api_requests: i64,
    pub total_response_time_ms: i64,
    pub timed_requests: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyMetadata {
    pub key_alias: Option<String>,
    pub team_id: Option<String>,
    pub user_id: Option<String>,
    pub user_email: Option<String>,
    pub key_exists: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyMetricWithMetadata {
    pub metrics: SpendMetrics,
    #[serde(default)]
    pub metadata: KeyMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricWithMetadata {
    pub metrics: SpendMetrics,
    #[serde(default)]
    pub metadata: IndexMap<String, Json>,
    #[serde(default)]
    pub api_key_breakdown: IndexMap<String, KeyMetricWithMetadata>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BreakdownMetrics {
    pub mcp_servers: IndexMap<String, MetricWithMetadata>,
    pub models: IndexMap<String, MetricWithMetadata>,
    pub model_groups: IndexMap<String, MetricWithMetadata>,
    pub providers: IndexMap<String, MetricWithMetadata>,
    pub endpoints: IndexMap<String, MetricWithMetadata>,
    pub api_keys: IndexMap<String, KeyMetricWithMetadata>,
    pub entities: IndexMap<String, MetricWithMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DailySpendData {
    pub date: NaiveDate,
    pub metrics: SpendMetrics,
    #[serde(default)]
    pub breakdown: BreakdownMetrics,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DailySpendMetadata {
    #[serde(with = "rust_decimal::serde::float")]
    pub total_spend: Decimal,
    #[serde(with = "rust_decimal::serde::float")]
    pub total_flat_cost: Decimal,
    pub total_prompt_tokens: i64,
    pub total_completion_tokens: i64,
    pub total_tokens: i64,
    pub total_api_requests: i64,
    pub total_successful_requests: i64,
    pub total_failed_requests: i64,
    pub total_cache_read_input_tokens: i64,
    #[serde(rename = "total_cache_creation_input_tokens")]
    pub total_cache_creation_tokens: i64,
    pub total_compression_saved_tokens: i64,
    #[serde(
        rename = "total_compression_savings_spend",
        with = "rust_decimal::serde::float"
    )]
    pub total_compression_savings: Decimal,
    #[serde(
        rename = "total_prompt_caching_savings_spend",
        with = "rust_decimal::serde::float"
    )]
    pub total_prompt_caching_savings: Decimal,
    #[serde(
        rename = "total_gateway_injected_caching_savings_spend",
        with = "rust_decimal::serde::float"
    )]
    pub total_gateway_caching_savings: Decimal,
    #[serde(with = "rust_decimal::serde::float")]
    pub total_autorouter_savings_spend: Decimal,
    pub total_response_time_ms: i64,
    pub total_timed_requests: i64,
    pub page: i64,
    pub total_pages: i64,
    pub has_more: bool,
    pub api_key_limit: Option<i64>,
    pub total_api_keys: Option<i64>,
    pub entity_total_api_keys: Option<IndexMap<String, i64>>,
}

impl Default for DailySpendMetadata {
    fn default() -> Self {
        Self {
            total_spend: Decimal::ZERO,
            total_flat_cost: Decimal::ZERO,
            total_prompt_tokens: 0,
            total_completion_tokens: 0,
            total_tokens: 0,
            total_api_requests: 0,
            total_successful_requests: 0,
            total_failed_requests: 0,
            total_cache_read_input_tokens: 0,
            total_cache_creation_tokens: 0,
            total_compression_saved_tokens: 0,
            total_compression_savings: Decimal::ZERO,
            total_prompt_caching_savings: Decimal::ZERO,
            total_gateway_caching_savings: Decimal::ZERO,
            total_autorouter_savings_spend: Decimal::ZERO,
            total_response_time_ms: 0,
            total_timed_requests: 0,
            page: 1,
            total_pages: 1,
            has_more: false,
            api_key_limit: None,
            total_api_keys: None,
            entity_total_api_keys: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename = "SpendAnalyticsPaginatedResponse")]
pub struct DailySpendPage {
    pub results: Vec<DailySpendData>,
    #[serde(default)]
    pub metadata: DailySpendMetadata,
}

mod tests;
