use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

// autorouter_savings_spend, total_response_time_ms and timed_requests are NotRequired in _types.py, so a row queued before a rolling upgrade can omit them.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaseDailySpendTransaction {
    pub date: String,
    pub api_key: String,
    pub model: Option<String>,
    pub model_group: Option<String>,
    pub mcp_namespaced_tool_name: Option<String>,
    pub custom_llm_provider: Option<String>,
    pub endpoint: Option<String>,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub cache_read_input_tokens: i64,
    pub cache_creation_input_tokens: i64,
    pub compression_saved_tokens: i64,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub compression_savings_spend: Decimal,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub prompt_caching_savings_spend: Decimal,
    #[serde(rename = "gateway_injected_caching_savings_spend")]
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub gateway_cache_savings_spend: Decimal,
    #[serde(default)]
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub autorouter_savings_spend: Decimal,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub spend: Decimal,
    pub api_requests: i64,
    pub successful_requests: i64,
    pub failed_requests: i64,
    #[serde(default)]
    pub total_response_time_ms: i64,
    #[serde(default)]
    pub timed_requests: i64,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DailyTeamSpendTransaction {
    #[serde(flatten)]
    pub base: BaseDailySpendTransaction,
    pub team_id: String,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DailyOrgSpendTransaction {
    #[serde(flatten)]
    pub base: BaseDailySpendTransaction,
    pub organization_id: String,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DailyUserSpendTransaction {
    #[serde(flatten)]
    pub base: BaseDailySpendTransaction,
    pub user_id: String,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DailyEndUserSpendTransaction {
    #[serde(flatten)]
    pub base: BaseDailySpendTransaction,
    pub end_user_id: String,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DailyTagSpendTransaction {
    #[serde(flatten)]
    pub base: BaseDailySpendTransaction,
    pub request_id: Option<String>,
    pub tag: String,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DailyAgentSpendTransaction {
    #[serde(flatten)]
    pub base: BaseDailySpendTransaction,
    pub agent_id: String,
}
