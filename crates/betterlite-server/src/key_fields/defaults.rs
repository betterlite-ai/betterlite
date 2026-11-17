pub(crate) const BASE_DEFAULTS: &str = r#"{"key_alias":null,"duration":null,"models":[],"spend":0.0,"max_budget":null,"user_id":null,"team_id":null,"agent_id":null,"max_parallel_requests":null,"metadata":{},"tpm_limit":null,"rpm_limit":null,"budget_duration":null,"budget_limits":null,"allowed_cache_controls":[],"config":{},"permissions":{},"model_max_budget":{},"budget_fallbacks":null,"model_rpm_limit":null,"model_tpm_limit":null,"mcp_rpm_limit":null,"tag_rpm_limit":null,"guardrails":null,"policies":null,"prompts":null,"blocked":null,"aliases":{},"object_permission":null}"#;

pub(crate) const KEY_DEFAULTS: &str = r#"{"key":null,"tpd_limit":null,"default_estimated_output_tokens":null,"default_estimated_output_tokens_per_model":null,"budget_id":null,"end_user_budget_id":null,"tags":null,"disable_global_guardrails":null,"enable_prompt_caching":null,"throttle_on_budget_exceeded":null,"enforced_params":null,"allowed_routes":[],"allowed_passthrough_routes":null,"allowed_vector_store_indexes":null,"rpm_limit_type":null,"tpm_limit_type":null,"router_settings":null,"access_group_ids":null}"#;

pub(crate) const GENERATE_DEFAULTS: &str = r#"{"soft_budget":null,"send_invite_email":null,"key_type":"default","auto_rotate":false,"rotation_interval":null,"organization_id":null,"project_id":null}"#;

pub(crate) const UPDATE_DEFAULTS: &str = r#"{"duration":null,"spend":null,"metadata":null,"soft_budget":null,"temp_budget_increase":null,"temp_budget_expiry":null,"auto_rotate":null,"rotation_interval":null,"organization_id":null,"project_id":null}"#;

pub(crate) const REGENERATE_DEFAULTS: &str = r#"{"duration":null,"spend":null,"metadata":null,"new_key":null,"new_master_key":null,"grace_period":null}"#;

pub(crate) const RESPONSE_DEFAULTS: &str = r#"{"key_name":null,"key_type":null,"expires":null,"token_id":null,"litellm_budget_table":null,"token":null,"created_by":null,"updated_by":null,"created_at":null,"updated_at":null,"organization_id":null,"project_id":null}"#;

pub(crate) const BROADCAST_DEFAULTS: &str = r#"{"max_budget":null,"budget_id":null,"budget_duration":null,"budget_limits":null,"model_max_budget":null,"tpm_limit":null,"rpm_limit":null,"model_tpm_limit":null,"model_rpm_limit":null,"max_parallel_requests":null,"rpm_limit_type":null,"tpm_limit_type":null,"temp_budget_increase":null,"temp_budget_expiry":null,"duration":null,"tags":null,"metadata":null}"#;

pub(crate) const OBJECT_PERMISSION_DEFAULTS: &str = r#"{"mcp_servers":null,"mcp_access_groups":null,"mcp_tool_permissions":null,"mcp_toolsets":null,"blocked_tools":null,"vector_stores":null,"agents":null,"agent_access_groups":null,"models":null,"search_tools":null,"mcp_tool_search_enabled":null,"skills":null}"#;
