use betterlite_config::sections::settings::JsonValue;

use super::super::validation::{Check, in_set, int_dict, is_integer, str_list};

#[derive(Clone, Copy)]
pub(super) enum Presence {
    Optional,
    Nullable,
    Required,
}

pub(super) type Rule = (&'static str, Check, Presence);

pub(super) const USER_ROLES: [&str; 2] = ["internal_user", "customer"];
pub(super) const BILLING_MODES: [&str; 2] = ["PROVISIONED_THROUGHPUT", "PAY_PER_REQUEST"];
pub(super) const VERSIONS: [&str; 4] = ["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"];
pub(super) const HEADER_KEYS: [&str; 2] = ["header_name", "litellm_user_role"];
pub(super) const STAGGER_KEYS: [&str; 4] = ["enabled", "window_seconds", "identity", "offsets"];
pub(super) const SPEND_KEYS: [&str; 4] = [
    "providers",
    "threshold",
    "lookback_days",
    "openai_project_ids",
];

pub(super) const ENDPOINT_RULES: [Rule; 12] = [
    ("id", JsonValue::is_string, Presence::Nullable),
    ("path", JsonValue::is_string, Presence::Required),
    ("target", JsonValue::is_string, Presence::Required),
    ("headers", JsonValue::is_object, Presence::Optional),
    (
        "default_query_params",
        JsonValue::is_object,
        Presence::Optional,
    ),
    ("include_subpath", JsonValue::is_boolean, Presence::Optional),
    ("cost_per_request", JsonValue::is_number, Presence::Optional),
    ("timeout", JsonValue::is_number, Presence::Nullable),
    ("auth", JsonValue::is_boolean, Presence::Optional),
    ("guardrails", super::is_guardrails, Presence::Nullable),
    ("is_from_config", JsonValue::is_boolean, Presence::Optional),
    ("methods", str_list, Presence::Nullable),
];
pub(super) const GUARDRAIL_RULES: [Rule; 2] = [
    ("request_fields", str_list, Presence::Nullable),
    ("response_fields", str_list, Presence::Nullable),
];
pub(super) const REDIS_NODE_RULES: [Rule; 2] = [
    ("host", JsonValue::is_string, Presence::Required),
    ("port", is_integer, Presence::Required),
];
pub(super) const HEADER_RULES: [Rule; 2] = [
    ("header_name", JsonValue::is_string, Presence::Required),
    (
        "litellm_user_role",
        |value| in_set(value, &USER_ROLES),
        Presence::Required,
    ),
];
pub(super) const CLIENT_RULES: [Rule; 2] = [
    (
        "alias",
        |value| value.as_str().is_some_and(|text| !text.is_empty()),
        Presence::Required,
    ),
    (
        "value",
        |value| value.as_str().is_some_and(|text| !text.is_empty()),
        Presence::Required,
    ),
];
pub(super) const STAGGER_RULES: [Rule; 4] = [
    ("enabled", JsonValue::is_boolean, Presence::Optional),
    (
        "window_seconds",
        |value| is_integer(value) && value.as_f64().is_some_and(|numeric| numeric >= 0.0),
        Presence::Optional,
    ),
    ("identity", JsonValue::is_string, Presence::Nullable),
    ("offsets", int_dict, Presence::Optional),
];
pub(super) const SPEND_RULES: [Rule; 2] = [
    ("providers", super::non_empty_openai, Presence::Optional),
    ("openai_project_ids", str_list, Presence::Optional),
];
pub(super) const DYNAMO_RULES: [Rule; 18] = [
    (
        "billing_mode",
        |value| in_set(value, &BILLING_MODES),
        Presence::Required,
    ),
    ("read_capacity_units", is_integer, Presence::Nullable),
    ("write_capacity_units", is_integer, Presence::Nullable),
    ("ssl_verify", JsonValue::is_boolean, Presence::Nullable),
    ("region_name", JsonValue::is_string, Presence::Required),
    ("user_table_name", JsonValue::is_string, Presence::Optional),
    ("key_table_name", JsonValue::is_string, Presence::Optional),
    (
        "config_table_name",
        JsonValue::is_string,
        Presence::Optional,
    ),
    ("spend_table_name", JsonValue::is_string, Presence::Optional),
    ("aws_role_name", JsonValue::is_string, Presence::Nullable),
    ("aws_session_name", JsonValue::is_string, Presence::Nullable),
    (
        "aws_web_identity_token",
        JsonValue::is_string,
        Presence::Nullable,
    ),
    ("aws_provider_id", JsonValue::is_string, Presence::Nullable),
    ("aws_policy_arns", str_list, Presence::Nullable),
    ("aws_policy", JsonValue::is_string, Presence::Nullable),
    ("aws_duration_seconds", is_integer, Presence::Nullable),
    (
        "assume_role_aws_role_name",
        JsonValue::is_string,
        Presence::Nullable,
    ),
    (
        "assume_role_aws_session_name",
        JsonValue::is_string,
        Presence::Nullable,
    ),
];
pub(super) const REDIS_RULES: [Rule; 15] = [
    ("host", JsonValue::is_string, Presence::Nullable),
    ("port", is_integer, Presence::Nullable),
    ("password", JsonValue::is_string, Presence::Nullable),
    ("username", JsonValue::is_string, Presence::Nullable),
    ("url", JsonValue::is_string, Presence::Nullable),
    ("ssl", JsonValue::is_boolean, Presence::Nullable),
    ("startup_nodes", super::is_redis_nodes, Presence::Nullable),
    (
        "sentinel_nodes",
        super::is_sentinel_nodes,
        Presence::Nullable,
    ),
    (
        "sentinel_password",
        JsonValue::is_string,
        Presence::Nullable,
    ),
    ("service_name", JsonValue::is_string, Presence::Nullable),
    (
        "aws_iam_auth",
        |value| value.is_boolean() || value.is_string(),
        Presence::Nullable,
    ),
    (
        "aws_iam_user_name",
        JsonValue::is_string,
        Presence::Nullable,
    ),
    (
        "aws_iam_cache_name",
        JsonValue::is_string,
        Presence::Nullable,
    ),
    ("aws_iam_region", JsonValue::is_string, Presence::Nullable),
    (
        "aws_iam_serverless",
        |value| value.is_boolean() || value.is_string(),
        Presence::Nullable,
    ),
];
