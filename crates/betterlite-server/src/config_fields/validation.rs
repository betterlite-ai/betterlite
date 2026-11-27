use crate::plugin_routes::config::PluginConfig;
use betterlite_config::sections::alert_settings::types::AlertType;
use betterlite_config::sections::settings::JsonValue;

use super::error::ConfigFieldError;
use super::models;

mod alerting;

pub(super) use alerting::validate_alerting_args;

pub(super) type Check = fn(&JsonValue) -> bool;

const STR: &str = "completion_model master_key control_plane_url database_url custom_auth user_header_name maximum_spend_logs_retention_period maximum_autorouter_session_retention_period maximum_health_check_retention_period maximum_daily_tag_spend_retention_period maximum_spend_logs_cleanup_run_budget maximum_spend_logs_cleanup_batch_timeout mcp_client_id_header";
const INT: &str = "database_connection_pool_limit max_parallel_requests global_max_parallel_requests max_request_size_mb max_batch_file_size_mb max_file_size_mb max_response_size_mb health_check_concurrency alerting_threshold password_policy_min_length maximum_spend_logs_cleanup_batch_size maximum_spend_logs_cleanup_max_batches";
const INT_POSITIVE: &str = "max_in_flight_requests_per_worker user_api_key_cache_max_size";
const INT_NON_NEGATIVE: &str = "max_queued_requests_per_worker";
const INT_AT_LEAST_ONE: &str = "max_failed_login_attempts_per_source failed_login_window_seconds failed_login_block_seconds mcp_xff_num_trusted_hops";
const INT_REQUIRED: &str = "health_check_interval";
const INT_REQUIRED_POSITIVE: &str = "proxy_config_reload_interval_seconds";
const FLOAT: &str = "database_connection_timeout database_connect_timeout database_socket_timeout database_max_idle_connection_lifetime pass_through_request_timeout";
const FLOAT_REQUIRED: &str = "admission_queue_timeout_seconds";
const BOOL: &str = "use_google_kms use_azure_key_vault dangerously_permit_weak_or_unset_master_key fail_closed_rate_limit_enforcement allow_cli_sso_verification_uri_complete include_call_id_in_error_body enable_claude_code_gateway database_disable_prepared_statements otel cancel_on_disconnect infer_model_from_keys background_health_checks model_list_healthy_only reject_clientside_metadata_tags enable_openai_websocket_passthrough store_prompts_in_spend_logs disable_auto_add_proxy_admin_to_teams enforce_fallback_model_access use_spend_logs_partitioning store_model_in_db forward_client_headers_to_llm_api password_policy_require_uppercase password_policy_require_lowercase password_policy_require_numbers password_policy_require_special_characters disable_password_login_when_sso_enabled disable_responses_id_security allow_unmanaged_response_ids disable_env_credential_login disable_budget_reservation apply_user_budget_to_team_keys user_url_validation";
const BOOL_REQUIRED: &str = "health_check_skip_disabled_background_models enable_public_model_hub";
const DICT: &str =
    "claude_code_gateway_managed_settings database_extra_connection_params alert_to_webhook_url";
const INT_DICT: &str = "max_failed_login_attempts_per_source_overrides";
const LIST: &str = "alerting allowed_routes";
const STR_LIST: &str = "allowed_file_extensions blocked_file_extensions background_health_check_model_groups transcribe_media_buckets mcp_internal_ip_ranges mcp_trusted_proxy_ranges trusted_proxy_ranges mcp_required_fields user_url_allowed_hosts provider_url_destination_allowed_hosts";
const DB_OBJECTS: [&str; 11] = [
    "models",
    "mcp",
    "guardrails",
    "policies",
    "vector_stores",
    "pass_through_endpoints",
    "prompts",
    "model_cost_map",
    "tools",
    "config_overrides",
    "websearch_interception_settings",
];
const KMS: [&str; 9] = [
    "google_kms",
    "azure_key_vault",
    "aws_secret_manager",
    "google_secret_manager",
    "hashicorp_vault",
    "cyberark",
    "local",
    "aws_kms",
    "custom",
];

// ConfigGeneralSettings.model_fields: every declared key with its pydantic check, and whether it takes a null
const KINDS: [(&str, Check, bool); 32] = [
    (STR, JsonValue::is_string, true),
    (INT, is_integer, true),
    (
        INT_POSITIVE,
        |value| is_integer(value) && value.as_f64().is_some_and(|numeric| numeric > 0.0),
        true,
    ),
    (
        INT_NON_NEGATIVE,
        |value| is_integer(value) && value.as_f64().is_some_and(|numeric| numeric >= 0.0),
        true,
    ),
    (
        INT_AT_LEAST_ONE,
        |value| is_integer(value) && value.as_f64().is_some_and(|numeric| numeric >= 1.0),
        true,
    ),
    (INT_REQUIRED, is_integer, false),
    (
        INT_REQUIRED_POSITIVE,
        |value| is_integer(value) && value.as_f64().is_some_and(|numeric| numeric > 0.0),
        false,
    ),
    (FLOAT, JsonValue::is_number, true),
    (
        FLOAT_REQUIRED,
        |value| JsonValue::is_number(value) && value.as_f64().is_some_and(|numeric| numeric > 0.0),
        false,
    ),
    (BOOL, JsonValue::is_boolean, true),
    (BOOL_REQUIRED, JsonValue::is_boolean, false),
    (DICT, JsonValue::is_object, true),
    (INT_DICT, int_dict, true),
    (LIST, JsonValue::is_array, true),
    (STR_LIST, str_list, true),
    (
        "alert_types",
        |value| serde_json::from_value::<AlertType>(value.clone()).is_ok(),
        true,
    ),
    (
        "supported_db_objects",
        |value| values_in(value, &DB_OBJECTS),
        true,
    ),
    (
        "plugins",
        |value| serde_json::from_value::<Vec<PluginConfig>>(JsonValue::clone(value)).is_ok(),
        true,
    ),
    ("pass_through_endpoints", models::endpoints, true),
    ("alerting_args", JsonValue::is_object, true),
    ("user_header_mappings", models::header_mappings, true),
    ("mcp_advertised_versions", models::advertised_versions, true),
    ("mcp_allowed_clients", models::allowed_clients, true),
    ("database_args", models::database_args, true),
    ("coordination_redis", models::redis, true),
    ("scheduled_job_stagger", models::stagger, true),
    ("spend_capture_rate_check", models::spend_capture, true),
    ("key_management_system", |value| in_set(value, &KMS), true),
    ("database_type", |value| in_set(value, &["dynamo_db"]), true),
    (
        "ui_access_mode",
        |value| in_set(value, &["admin_only", "all"]),
        true,
    ),
    (
        "missing_session_id",
        |value| in_set(value, &["generate", "reject", "omit"]),
        true,
    ),
    (
        "user_mcp_management_mode",
        |value| in_set(value, &["restricted", "view_all"]),
        true,
    ),
];

// one declared type of ConfigGeneralSettings: the check that accepts a value, and whether a null is acceptable
#[derive(Clone, Copy)]
pub struct Kind {
    check: Check,
    nullable: bool,
}

#[must_use]
pub fn kind_of(name: &str) -> Option<Kind> {
    KINDS.iter().find_map(|(names, check, nullable)| {
        names
            .split_whitespace()
            .any(|candidate| candidate == name)
            .then_some(Kind {
                check: *check,
                nullable: *nullable,
            })
    })
}

// type(data.field_value), which is what LiteLLM writes into its Invalid-type sentence
#[must_use]
pub fn litellm_type_name(value: &JsonValue) -> &'static str {
    match value {
        JsonValue::Null => "<class 'NoneType'>",
        JsonValue::Bool(_) => "<class 'bool'>",
        JsonValue::Number(number) => {
            if number.as_i64().is_some() || number.as_u64().is_some() {
                "<class 'int'>"
            } else {
                "<class 'float'>"
            }
        }
        JsonValue::String(_) => "<class 'str'>",
        JsonValue::Array(_) => "<class 'list'>",
        JsonValue::Object(_) => "<class 'dict'>",
    }
}

// ConfigGeneralSettings.model_validate({field_name: value}), minus the coercion pydantic does silently
pub fn validate(kind: Kind, value: &JsonValue) -> Result<(), ConfigFieldError> {
    if value.is_null() {
        return if kind.nullable {
            Ok(())
        } else {
            Err(invalid_type(value))
        };
    }
    if (kind.check)(value) {
        return Ok(());
    }
    Err(invalid_type(value))
}

fn invalid_type(value: &JsonValue) -> ConfigFieldError {
    ConfigFieldError::BadRequest {
        detail: format!(
            "Invalid type of field value={} passed in.",
            litellm_type_name(value)
        ),
    }
}

#[must_use]
pub fn is_integer(value: &JsonValue) -> bool {
    value
        .as_f64()
        .is_some_and(|numeric| numeric.is_finite() && numeric.fract() == 0.0)
}

#[must_use]
pub fn int_dict(value: &JsonValue) -> bool {
    value
        .as_object()
        .is_some_and(|entries| entries.values().all(is_integer))
}

#[must_use]
pub fn str_list(value: &JsonValue) -> bool {
    value
        .as_array()
        .is_some_and(|items| items.iter().all(JsonValue::is_string))
}

fn values_in(value: &JsonValue, options: &[&str]) -> bool {
    value
        .as_array()
        .is_some_and(|items| items.iter().all(|item| in_set(item, options)))
}

#[must_use]
pub fn in_set(value: &JsonValue, options: &[&str]) -> bool {
    let Some(text) = value.as_str() else {
        return false;
    };
    options.contains(&text)
}
