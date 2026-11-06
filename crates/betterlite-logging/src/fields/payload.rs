use betterlite_models::cost_breakdown::CostBreakdown;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::fields::payload::records::{
    ErrorInformation, GuardrailInformation, ModelInformation, StandardLoggingHiddenParams,
    StandardLoggingMetadata,
};
use crate::fields::status::{LoggingStatus, StatusFields};

pub mod records;

mod tests;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StandardLoggingPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classifier_input: Option<IndexMap<String, Json>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub originating_request_masked: Option<IndexMap<String, Json>>,
    pub id: String,
    pub trace_id: String,
    pub session_id: String,
    pub litellm_call_id: Option<String>,
    pub call_type: String,
    pub stream: Option<bool>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub response_cost: Decimal,
    pub cost_breakdown: Option<CostBreakdown>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub autorouter_savings: Option<Decimal>,
    pub autorouter_savings_estimate: Option<IndexMap<String, Json>>,
    #[serde(rename = "autorouter_baseline_observation")]
    pub baseline_observation: Option<String>,
    #[serde(rename = "response_cost_failure_debug_info")]
    pub cost_failure_info: Option<Json>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zero_cost_diagnostic: Option<Json>,
    pub status: LoggingStatus,
    pub status_fields: StatusFields,
    pub custom_llm_provider: Option<String>,
    pub total_tokens: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    #[serde(
        rename = "startTime",
        with = "rust_decimal::serde::arbitrary_precision"
    )]
    pub start_time: Decimal,
    #[serde(rename = "endTime", with = "rust_decimal::serde::arbitrary_precision")]
    pub end_time: Decimal,
    #[serde(
        rename = "completionStartTime",
        with = "rust_decimal::serde::arbitrary_precision"
    )]
    pub completion_start_time: Decimal,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub response_time: Decimal,
    pub model_map_information: ModelInformation,
    pub model: String,
    pub model_id: Option<String>,
    pub model_group: Option<String>,
    pub api_base: String,
    pub metadata: StandardLoggingMetadata,
    pub cache_hit: Option<bool>,
    pub cache_key: Option<String>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub saved_cache_cost: Decimal,
    pub request_tags: Vec<Json>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_model_access_groups: Option<Vec<String>>,
    pub end_user: Option<String>,
    pub requester_ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub messages: Option<Json>,
    pub response: Option<Json>,
    pub error_str: Option<String>,
    pub error_information: Option<ErrorInformation>,
    pub model_parameters: IndexMap<String, Json>,
    pub hidden_params: StandardLoggingHiddenParams,
    pub guardrail_information: Option<Vec<GuardrailInformation>>,
    pub standard_built_in_tools_params: Option<Json>,
}
