use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_with::rust::double_option as nullable;

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StandardLoggingMetadata {
    pub user_api_key_hash: Option<String>,
    pub user_api_key_alias: Option<String>,
    pub user_api_key_spend: Option<DecimalValue>,
    pub user_api_key_max_budget: Option<DecimalValue>,
    pub user_api_key_budget_reset_at: Option<String>,
    pub user_api_key_user_spend: Option<DecimalValue>,
    pub user_api_key_user_max_budget: Option<DecimalValue>,
    pub user_api_key_team_spend: Option<DecimalValue>,
    pub user_api_key_team_max_budget: Option<DecimalValue>,
    pub user_api_key_org_id: Option<String>,
    pub user_api_key_org_alias: Option<String>,
    pub user_api_key_team_id: Option<String>,
    pub user_api_key_project_id: Option<String>,
    pub user_api_key_project_alias: Option<String>,
    pub user_api_key_user_id: Option<String>,
    pub user_api_key_user_email: Option<String>,
    pub user_api_key_team_alias: Option<String>,
    pub user_api_key_end_user_id: Option<String>,
    pub user_api_key_request_route: Option<String>,
    pub user_api_key_auth_metadata: Option<IndexMap<String, String>>,
    pub spend_logs_metadata: Option<Json>,
    pub requester_ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub requester_metadata: Option<Json>,
    pub requester_custom_headers: Option<IndexMap<String, String>>,
    pub prompt_management_metadata: Option<Json>,
    pub mcp_tool_call_metadata: Option<Json>,
    pub vector_store_request_metadata: Option<Vec<Json>>,
    pub routing_decision: Option<Json>,
    pub applied_guardrails: Option<Vec<String>>,
    pub usage_object: Option<Json>,
    pub cold_storage_object_key: Option<String>,
    pub team_alias: Option<String>,
    pub team_id: Option<String>,
    pub used_client_oauth_token: Option<bool>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StandardLoggingHiddenParams {
    pub model_id: Option<String>,
    pub cache_key: Option<String>,
    pub api_base: Option<String>,
    pub response_cost: Option<Json>,
    pub litellm_overhead_time_ms: Option<DecimalValue>,
    pub additional_headers: Option<AdditionalHeaders>,
    pub batch_models: Option<Vec<String>>,
    pub batch_successful_requests: Option<i64>,
    pub batch_failed_requests: Option<i64>,
    pub litellm_model_name: Option<String>,
    pub usage_object: Option<Json>,
}

pub type AdditionalHeaders = IndexMap<String, Json>;

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ErrorInformation {
    #[serde(default, with = "nullable", skip_serializing_if = "Option::is_none")]
    pub error_code: Option<Option<String>>,
    #[serde(default, with = "nullable", skip_serializing_if = "Option::is_none")]
    pub error_class: Option<Option<String>>,
    #[serde(default, with = "nullable", skip_serializing_if = "Option::is_none")]
    pub llm_provider: Option<Option<String>>,
    #[serde(default, with = "nullable", skip_serializing_if = "Option::is_none")]
    pub traceback: Option<Option<String>>,
    #[serde(default, with = "nullable", skip_serializing_if = "Option::is_none")]
    pub error_message: Option<Option<String>>,
    #[serde(default, with = "nullable", skip_serializing_if = "Option::is_none")]
    pub error_provider_request_id: Option<Option<String>>,
    #[serde(default, with = "nullable", skip_serializing_if = "Option::is_none")]
    pub error_rate_limit_category: Option<Option<String>>,
    #[serde(default, with = "nullable", skip_serializing_if = "Option::is_none")]
    pub error_rate_limit_type: Option<Option<String>>,
    #[serde(default, with = "nullable", skip_serializing_if = "Option::is_none")]
    pub error_budget_entity_type: Option<Option<String>>,
    #[serde(default, with = "nullable", skip_serializing_if = "Option::is_none")]
    pub error_budget_entity_id: Option<Option<String>>,
    #[serde(default, with = "nullable", skip_serializing_if = "Option::is_none")]
    pub normalized_error: Option<Option<String>>,
    #[serde(default, with = "nullable", skip_serializing_if = "Option::is_none")]
    pub error_budget_limit: Option<Option<DecimalValue>>,
    #[serde(default, with = "nullable", skip_serializing_if = "Option::is_none")]
    pub error_budget_spend: Option<Option<DecimalValue>>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GuardrailInformation {
    #[serde(default, with = "nullable", skip_serializing_if = "Option::is_none")]
    pub guardrail_name: Option<Option<String>>,
    #[serde(default, with = "nullable", skip_serializing_if = "Option::is_none")]
    pub guardrail_provider: Option<Option<String>>,
    #[serde(default, with = "nullable", skip_serializing_if = "Option::is_none")]
    pub guardrail_mode: Option<Option<Json>>,
    #[serde(default, with = "nullable", skip_serializing_if = "Option::is_none")]
    pub guardrail_request: Option<Option<Json>>,
    #[serde(default, with = "nullable", skip_serializing_if = "Option::is_none")]
    pub guardrail_response: Option<Option<Json>>,
    #[serde(default, with = "nullable", skip_serializing_if = "Option::is_none")]
    pub guardrail_status: Option<Option<String>>,
    #[serde(default, with = "nullable", skip_serializing_if = "Option::is_none")]
    pub start_time: Option<Option<DecimalValue>>,
    #[serde(default, with = "nullable", skip_serializing_if = "Option::is_none")]
    pub end_time: Option<Option<DecimalValue>>,
    #[serde(default, with = "nullable", skip_serializing_if = "Option::is_none")]
    pub duration: Option<Option<DecimalValue>>,
    #[serde(default, with = "nullable", skip_serializing_if = "Option::is_none")]
    pub masked_entity_count: Option<Option<IndexMap<String, i64>>>,
    #[serde(default, with = "nullable", skip_serializing_if = "Option::is_none")]
    pub guardrail_id: Option<Option<String>>,
    #[serde(default, with = "nullable", skip_serializing_if = "Option::is_none")]
    pub policy_template: Option<Option<String>>,
    #[serde(default, with = "nullable", skip_serializing_if = "Option::is_none")]
    pub detection_method: Option<Option<String>>,
    #[serde(default, with = "nullable", skip_serializing_if = "Option::is_none")]
    pub confidence_score: Option<Option<DecimalValue>>,
    #[serde(default, with = "nullable", skip_serializing_if = "Option::is_none")]
    pub classification: Option<Option<Json>>,
    #[serde(default, with = "nullable", skip_serializing_if = "Option::is_none")]
    pub match_details: Option<Option<Json>>,
    #[serde(default, with = "nullable", skip_serializing_if = "Option::is_none")]
    pub patterns_checked: Option<Option<i64>>,
    #[serde(default, with = "nullable", skip_serializing_if = "Option::is_none")]
    pub alert_recipients: Option<Option<Vec<String>>>,
    #[serde(default, with = "nullable", skip_serializing_if = "Option::is_none")]
    pub risk_score: Option<Option<DecimalValue>>,
    #[serde(default, with = "nullable", skip_serializing_if = "Option::is_none")]
    pub violation_categories: Option<Option<Vec<String>>>,
    #[serde(default, with = "nullable", skip_serializing_if = "Option::is_none")]
    pub guardrail_action: Option<Option<String>>,
    #[serde(default, with = "nullable", skip_serializing_if = "Option::is_none")]
    pub guardrail_usage: Option<Option<IndexMap<String, i64>>>,
    #[serde(default, with = "nullable", skip_serializing_if = "Option::is_none")]
    pub guardrail_cost: Option<Option<DecimalValue>>,
    #[serde(default, with = "nullable", skip_serializing_if = "Option::is_none")]
    pub guardrail_cost_by_unit: Option<Option<IndexMap<String, Option<DecimalValue>>>>,
    #[serde(default, with = "nullable", skip_serializing_if = "Option::is_none")]
    pub guardrail_cost_in_spend: Option<Option<bool>>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct DecimalValue(#[serde(with = "rust_decimal::serde::arbitrary_precision")] pub Decimal);
