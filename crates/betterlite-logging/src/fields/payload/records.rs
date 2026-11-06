use betterlite_models::catalog_generalizations::ModelInfo;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

// StandardLoggingMetadata: every key is logged, and an absent auth or spend value stays null, never a fake zero.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StandardLoggingMetadata {
    pub user_api_key_hash: Option<String>,
    pub user_api_key_alias: Option<String>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub user_api_key_spend: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub user_api_key_max_budget: Option<Decimal>,
    pub user_api_key_budget_reset_at: Option<String>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub user_api_key_user_spend: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub user_api_key_user_max_budget: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub user_api_key_team_spend: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub user_api_key_team_max_budget: Option<Decimal>,
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
    pub response_cost: Option<ResponseCost>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub litellm_overhead_time_ms: Option<Decimal>,
    pub additional_headers: Option<AdditionalHeaders>,
    pub batch_models: Option<Vec<String>>,
    pub batch_successful_requests: Option<i64>,
    pub batch_failed_requests: Option<i64>,
    pub litellm_model_name: Option<String>,
    pub usage_object: Option<Json>,
}

// The declared response_cost is str | float | None, so a text cost keeps its text.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ResponseCost {
    Text(String),
    Number(#[serde(with = "rust_decimal::serde::arbitrary_precision")] Decimal),
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct AdditionalHeaders {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x_ratelimit_limit_requests: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x_ratelimit_limit_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x_ratelimit_remaining_requests: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x_ratelimit_remaining_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x_ratelimit_reset_requests: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x_ratelimit_reset_tokens: Option<String>,
    #[serde(flatten)]
    pub extra: IndexMap<String, Json>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ModelInformation {
    pub model_map_key: String,
    pub model_map_value: Option<ModelInfo>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ErrorInformation {
    pub error_code: Option<String>,
    pub error_class: Option<String>,
    pub llm_provider: Option<String>,
    pub traceback: Option<String>,
    pub error_message: Option<String>,
    pub error_provider_request_id: Option<String>,
    pub error_rate_limit_category: Option<String>,
    pub error_rate_limit_type: Option<String>,
    pub error_budget_entity_type: Option<String>,
    pub error_budget_entity_id: Option<String>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub error_budget_limit: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub error_budget_spend: Option<Decimal>,
    pub normalized_error: Option<String>,
}

// StandardLoggingGuardrailInformation: a hook writes a partial record, and guardrail_status stays text for legacy values.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GuardrailInformation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guardrail_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guardrail_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guardrail_mode: Option<Json>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guardrail_request: Option<Json>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guardrail_response: Option<Json>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guardrail_status: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "rust_decimal::serde::arbitrary_precision_option"
    )]
    pub start_time: Option<Decimal>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "rust_decimal::serde::arbitrary_precision_option"
    )]
    pub end_time: Option<Decimal>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "rust_decimal::serde::arbitrary_precision_option"
    )]
    pub duration: Option<Decimal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub masked_entity_count: Option<IndexMap<String, i64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guardrail_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_template: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detection_method: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "rust_decimal::serde::arbitrary_precision_option"
    )]
    pub confidence_score: Option<Decimal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classification: Option<Json>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub match_details: Option<Json>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patterns_checked: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alert_recipients: Option<Vec<String>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "rust_decimal::serde::arbitrary_precision_option"
    )]
    pub risk_score: Option<Decimal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub violation_categories: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guardrail_action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guardrail_usage: Option<IndexMap<String, i64>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "rust_decimal::serde::arbitrary_precision_option"
    )]
    pub guardrail_cost: Option<Decimal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guardrail_cost_by_unit: Option<IndexMap<String, Option<Money>>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guardrail_cost_in_spend: Option<bool>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Money(#[serde(with = "rust_decimal::serde::arbitrary_precision")] pub Decimal); // litellm.types.utils.StandardBuiltInToolsParams; the tool option shapes belong to the OpenAI wire crate.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct BuiltInToolsParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web_search_options: Option<Json>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_search: Option<Json>,
}
