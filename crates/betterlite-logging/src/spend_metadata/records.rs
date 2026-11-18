use betterlite_models::decisions::Json;
use rust_decimal::Decimal;
use rust_decimal::serde::arbitrary_precision_option;
use serde::{Deserialize, Serialize};
use serde_with::rust::double_option as nullable;

// SpendLogsRouterMetadata: trusted router provenance, every field present and null when unknown.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct SpendLogsRouterMetadata {
    pub requested_model: Option<String>,
    pub selected_model: Option<String>,
    pub selected_provider: Option<String>,
    pub router_correlation_id: Option<String>,
}

// SpendLogsMetadata: one field per _types.py annotation, in declaration order.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct SpendLogsMetadata {
    // The five NotRequired agent fields and routing_decision are absent from the metadata=None constructor.
    #[serde(with = "nullable", skip_serializing_if = "Option::is_none")]
    pub actor_agent_id: Option<Option<String>>,
    #[serde(with = "nullable", skip_serializing_if = "Option::is_none")]
    pub target_agent_id: Option<Option<String>>,
    #[serde(with = "nullable", skip_serializing_if = "Option::is_none")]
    pub billing_agent_id: Option<Option<String>>,
    #[serde(with = "nullable", skip_serializing_if = "Option::is_none")]
    pub agent_execution_mode: Option<Option<String>>,
    #[serde(with = "nullable", skip_serializing_if = "Option::is_none")]
    pub verified_human_user_id: Option<Option<String>>,
    #[serde(rename = "autorouter_baseline_observation")]
    pub baseline_observation: Option<String>,
    pub additional_usage_values: Option<Json>,
    #[serde(rename = "user_api_key")]
    pub hashed_user_api_key: Option<String>,
    pub user_api_key_alias: Option<String>,
    pub user_api_key_team_id: Option<String>,
    pub user_api_key_project_id: Option<String>,
    pub user_api_key_project_alias: Option<String>,
    pub user_api_key_org_id: Option<String>,
    pub user_api_key_user_id: Option<String>,
    pub user_api_key_team_alias: Option<String>,
    pub spend_logs_metadata: Option<Json>,
    pub requester_ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub litellm_call_id: Option<String>,
    pub applied_guardrails: Option<Vec<String>>,
    pub mcp_tool_call_metadata: Option<Json>,
    pub vector_store_request_metadata: Option<Vec<Json>>,
    #[serde(with = "nullable", skip_serializing_if = "Option::is_none")]
    pub routing_decision: Option<Option<Json>>,
    pub internal_call_origin: Option<String>,
    pub guardrail_information: Option<Json>,
    pub eval_information: Option<Json>,
    pub status: Option<String>,
    pub proxy_server_request: Option<String>,
    pub batch_models: Option<Vec<String>>,
    pub batch_successful_requests: Option<i64>,
    pub batch_failed_requests: Option<i64>,
    pub error_information: Option<Json>,
    pub usage_object: Option<Json>,
    pub model_map_information: Option<Json>,
    pub cold_storage_object_key: Option<String>,
    #[serde(default, with = "arbitrary_precision_option")]
    pub litellm_overhead_time_ms: Option<Decimal>,
    pub attempted_retries: Option<i64>,
    pub max_retries: Option<i64>,
    pub attempted_fallbacks: Option<i64>,
    pub original_model_group: Option<String>,
    pub cost_breakdown: Option<Json>,
    pub compression_savings: Option<Json>,
    #[serde(default, with = "arbitrary_precision_option")]
    pub autorouter_savings: Option<Decimal>,
    pub autorouter_savings_estimate: Option<Json>,
    pub litellm_gateway_injected_cache: Option<String>,
    pub router_metadata: Option<SpendLogsRouterMetadata>,
    pub azure_spillover: Option<Json>,
    pub used_client_oauth_token: Option<bool>,
}

#[path = "records/tests.rs"]
mod tests;
