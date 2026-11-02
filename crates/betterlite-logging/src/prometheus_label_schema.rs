use crate::prometheus::request_label_catalog;
use betterlite_prometheus::caller_identity_logic::{self, CallerIdentityError};
use betterlite_prometheus::label_sanitization::sanitize_label_name;

mod tests;

// split from PrometheusLabelSettings so neither struct holds more than clippy's two-bool cap
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BudgetAndLengthToggles {
    pub emit_input_sequence_length: bool,
    pub include_email_alias: bool,
}

// B002 resolves and supplies these; get_labels only reads them
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PrometheusLabelSettings {
    pub caller_identity: String,
    pub emit_stream_label: bool,
    pub emit_rate_limit_labels: bool,
    pub budget_and_length: BudgetAndLengthToggles,
    pub custom_metadata_labels: Vec<String>,
    pub custom_tags: Vec<String>,
}

impl PrometheusLabelSettings {
    #[must_use]
    pub fn with_defaults() -> Self {
        Self {
            caller_identity: "api_key_alias".to_owned(),
            emit_stream_label: false,
            emit_rate_limit_labels: false,
            budget_and_length: BudgetAndLengthToggles {
                emit_input_sequence_length: false,
                include_email_alias: false,
            },
            custom_metadata_labels: Vec::new(),
            custom_tags: Vec::new(),
        }
    }
}

#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum LabelSchemaError {
    #[error("unknown prometheus metric: {metric}")]
    UnknownMetric { metric: String },
    #[error(transparent)]
    CallerIdentity(#[from] CallerIdentityError),
}

const USER_BUDGET_METRICS: [&str; 3] = [
    "litellm_remaining_user_budget_metric",
    "litellm_user_max_budget_metric",
    "litellm_user_budget_remaining_hours_metric",
];

// space-separated so rustfmt keeps each group on one line instead of one array item per line
fn label_set(space_separated: &str) -> Vec<String> {
    if space_separated.is_empty() {
        return Vec::new();
    }
    space_separated.split(' ').map(str::to_owned).collect()
}

const TEAM_BUDGET: &str = "team team_alias";
const ORG_BUDGET: &str = "org_id org_alias";
const API_KEY_BUDGET: &str = "hashed_api_key api_key_alias";
const USER_BUDGET: &str = "user";
const CUSTOMER_BUDGET: &str = "end_user";
const LIMIT_FOR_MODEL: &str = "hashed_api_key api_key_alias model model_id";
const CALLBACK: &str = "callback_name";
const API_KEY_RATE_LIMIT: &str = "hashed_api_key api_key_alias rate_limit_type";
const TEAM_RATE_LIMIT: &str = "team team_alias rate_limit_type";
const LLM_API_FAILED: &str =
    "end_user hashed_api_key api_key_alias model team team_alias user model_id";
const EMPTY: &str = "";
const CACHE: &str =
    "model hashed_api_key api_key_alias team team_alias end_user user model_id api_provider";
const BATCH_USER: &str = "model api_provider user user_email api_key_alias";
const BATCH_DURATION: &str = "model api_provider";
const STREAM_LABEL: &str = "stream";
const RATE_LIMIT_LABELS: &str = "rate_limit_category rate_limit_type";
const EMAIL_ALIAS_LABELS: &str = "user_email user_alias";
const INPUT_SEQUENCE_LABEL: &str = "input_sequence_length";
const MCP: &str =
    "mcp_tool_name mcp_server_name hashed_api_key api_key_alias team team_alias user end_user";
const DEPLOYMENT_TOTAL: &str = "model_group requested_model litellm_model_name model_id api_base api_provider hashed_api_key api_key_alias team team_alias client_ip user_agent";
const DEPLOYMENT_FAILURE: &str = "model_group requested_model litellm_model_name model_id api_base api_provider exception_status exception_class hashed_api_key api_key_alias team team_alias client_ip user_agent";

// PrometheusMetricLabels assignments, LiteLLM lines 701-955 (B3871 covers the rest); metric names are space-separated, like the label strings above, to keep one group per line
const STATIC_LABEL_GROUPS: &[(&str, &str)] = &[
    (
        "litellm_remaining_team_budget_metric litellm_team_max_budget_metric litellm_team_budget_remaining_hours_metric litellm_team_members_metric",
        TEAM_BUDGET,
    ),
    (
        "litellm_remaining_org_budget_metric litellm_org_max_budget_metric litellm_org_budget_remaining_hours_metric",
        ORG_BUDGET,
    ),
    (
        "litellm_remaining_api_key_budget_metric litellm_api_key_max_budget_metric litellm_api_key_budget_remaining_hours_metric",
        API_KEY_BUDGET,
    ),
    (
        "litellm_remaining_user_budget_metric litellm_user_max_budget_metric litellm_user_budget_remaining_hours_metric",
        USER_BUDGET,
    ),
    (
        "litellm_remaining_customer_budget_metric litellm_customer_max_budget_metric litellm_customer_budget_remaining_hours_metric",
        CUSTOMER_BUDGET,
    ),
    (
        "litellm_remaining_api_key_requests_for_model litellm_remaining_api_key_tokens_for_model",
        LIMIT_FOR_MODEL,
    ),
    ("litellm_callback_logging_failures_metric", CALLBACK),
    (
        "litellm_api_key_rate_limit_allowed_metric litellm_api_key_rate_limit_used_metric",
        API_KEY_RATE_LIMIT,
    ),
    (
        "litellm_team_rate_limit_allowed_metric litellm_team_rate_limit_used_metric",
        TEAM_RATE_LIMIT,
    ),
    ("litellm_llm_api_failed_requests_metric", LLM_API_FAILED),
    (
        "litellm_pod_lock_manager_size litellm_in_memory_daily_spend_update_queue_size litellm_redis_daily_spend_update_queue_size litellm_in_memory_spend_update_queue_size litellm_redis_spend_update_queue_size litellm_managed_file_size_bytes litellm_managed_file_deleted_total litellm_check_batch_cost_jobs_polled litellm_check_batch_cost_errors_total litellm_check_batch_cost_last_run_timestamp",
        EMPTY,
    ),
    (
        "litellm_cache_hits_metric litellm_cache_misses_metric litellm_cached_tokens_metric litellm_provider_cache_read_input_tokens_metric litellm_provider_cache_creation_input_tokens_metric",
        CACHE,
    ),
    (
        "litellm_managed_batch_created_total litellm_managed_file_created_total",
        BATCH_USER,
    ),
    (
        "litellm_managed_batch_duration_seconds litellm_check_batch_cost_jobs_processed_total",
        BATCH_DURATION,
    ),
    (
        "litellm_mcp_tool_calls_total litellm_mcp_tool_call_spend_metric",
        MCP,
    ),
    (
        "litellm_deployment_total_requests litellm_deployment_success_responses",
        DEPLOYMENT_TOTAL,
    ),
    ("litellm_deployment_failure_responses", DEPLOYMENT_FAILURE),
];

fn static_label_set(metric: &str) -> Option<&'static str> {
    STATIC_LABEL_GROUPS
        .iter()
        .find(|(metrics, _)| metrics.split(' ').any(|name| name == metric))
        .map(|(_, labels)| *labels)
}

fn is_org_label_metric(metric: &str) -> bool {
    matches!(
        metric,
        "litellm_llm_api_latency_metric"
            | "litellm_llm_api_time_to_first_token_metric"
            | "litellm_request_total_latency_metric"
            | "litellm_request_queue_time_seconds"
            | "litellm_proxy_total_requests_metric"
            | "litellm_proxy_failed_requests_metric"
            | "litellm_deployment_latency_per_output_token"
            | "litellm_requests_metric"
            | "litellm_spend_metric"
            | "litellm_input_tokens_metric"
            | "litellm_total_tokens_metric"
            | "litellm_output_tokens_metric"
            | "litellm_video_duration_seconds_metric"
            | "litellm_images_generated_metric"
    )
}

fn is_input_sequence_metric(metric: &str) -> bool {
    matches!(
        metric,
        "litellm_llm_api_latency_metric"
            | "litellm_llm_api_time_to_first_token_metric"
            | "litellm_request_total_latency_metric"
    )
}

// None means no declared schema, e.g. litellm_in_flight_requests (docs/deviations/B3870.yaml)
fn default_labels(metric: &str) -> Option<Vec<String>> {
    if let Some(labels) = request_label_catalog::labels_for_metric(metric) {
        return Some(labels.iter().map(|label| (*label).to_owned()).collect());
    }
    Some(label_set(static_label_set(metric)?))
}

fn push_if_absent(resolved: &[String], custom_labels: &mut Vec<String>, label: &str) {
    if !resolved.iter().any(|existing| existing == label)
        && !custom_labels.iter().any(|existing| existing == label)
    {
        custom_labels.push(label.to_owned());
    }
}

fn custom_prefix_labels(settings: &PrometheusLabelSettings) -> Vec<String> {
    let mut labels: Vec<String> = settings
        .custom_metadata_labels
        .iter()
        .map(|label| sanitize_label_name(label))
        .collect();
    labels.extend(
        settings
            .custom_tags
            .iter()
            .map(|tag| sanitize_label_name(&format!("tag_{tag}"))),
    );
    labels
}

fn opt_in_groups(metric: &str, settings: &PrometheusLabelSettings) -> [(bool, &'static str); 5] {
    [
        (
            metric == "litellm_proxy_total_requests_metric" && settings.emit_stream_label,
            STREAM_LABEL,
        ),
        (
            metric == "litellm_proxy_failed_requests_metric" && settings.emit_rate_limit_labels,
            RATE_LIMIT_LABELS,
        ),
        (
            USER_BUDGET_METRICS.contains(&metric) && settings.budget_and_length.include_email_alias,
            EMAIL_ALIAS_LABELS,
        ),
        (is_org_label_metric(metric), ORG_BUDGET),
        (
            is_input_sequence_metric(metric)
                && settings.budget_and_length.emit_input_sequence_length,
            INPUT_SEQUENCE_LABEL,
        ),
    ]
}

// returns a fresh Vec per call, so a caller mutating the result can't corrupt another metric's schema
pub fn get_labels(
    metric: &str,
    settings: &PrometheusLabelSettings,
) -> Result<Vec<String>, LabelSchemaError> {
    let defaults = default_labels(metric).ok_or_else(|| LabelSchemaError::UnknownMetric {
        metric: metric.to_owned(),
    })?;
    let resolved = caller_identity_logic::resolve_caller_identity_labels(
        metric,
        &defaults,
        Some(&settings.caller_identity),
    )?;
    let mut custom_labels = custom_prefix_labels(settings);
    for (condition, labels) in opt_in_groups(metric, settings) {
        if condition {
            for label in label_set(labels) {
                push_if_absent(&resolved, &mut custom_labels, &label);
            }
        }
    }
    let mut labels = resolved;
    labels.extend(custom_labels);
    Ok(labels)
}
