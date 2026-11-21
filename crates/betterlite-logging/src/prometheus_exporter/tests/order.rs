use betterlite_prometheus::collector_registry::metric::CollectorKind;

use crate::prometheus_exporter::PrometheusExporterSettings;

use super::{default_label_settings, registered};

const EXPECTED: [(&str, CollectorKind); 81] = [
    (
        "litellm_proxy_failed_requests_metric",
        CollectorKind::Counter,
    ),
    (
        "litellm_proxy_total_requests_metric",
        CollectorKind::Counter,
    ),
    (
        "litellm_request_total_latency_metric",
        CollectorKind::Histogram,
    ),
    ("litellm_llm_api_latency_metric", CollectorKind::Histogram),
    (
        "litellm_llm_api_time_to_first_token_metric",
        CollectorKind::Histogram,
    ),
    ("litellm_spend_metric", CollectorKind::Counter),
    ("litellm_total_tokens_metric", CollectorKind::Counter),
    ("litellm_input_tokens_metric", CollectorKind::Counter),
    ("litellm_output_tokens_metric", CollectorKind::Counter),
    ("litellm_input_cached_tokens_metric", CollectorKind::Counter),
    (
        "litellm_input_cache_creation_tokens_metric",
        CollectorKind::Counter,
    ),
    ("litellm_input_audio_tokens_metric", CollectorKind::Counter),
    (
        "litellm_output_reasoning_tokens_metric",
        CollectorKind::Counter,
    ),
    ("litellm_output_audio_tokens_metric", CollectorKind::Counter),
    (
        "litellm_video_duration_seconds_metric",
        CollectorKind::Counter,
    ),
    ("litellm_images_generated_metric", CollectorKind::Counter),
    ("litellm_remaining_team_budget_metric", CollectorKind::Gauge),
    ("litellm_team_max_budget_metric", CollectorKind::Gauge),
    (
        "litellm_team_budget_remaining_hours_metric",
        CollectorKind::Gauge,
    ),
    ("litellm_team_members_metric", CollectorKind::Gauge),
    ("litellm_remaining_org_budget_metric", CollectorKind::Gauge),
    ("litellm_org_max_budget_metric", CollectorKind::Gauge),
    (
        "litellm_org_budget_remaining_hours_metric",
        CollectorKind::Gauge,
    ),
    (
        "litellm_remaining_api_key_budget_metric",
        CollectorKind::Gauge,
    ),
    ("litellm_api_key_max_budget_metric", CollectorKind::Gauge),
    (
        "litellm_api_key_budget_remaining_hours_metric",
        CollectorKind::Gauge,
    ),
    ("litellm_remaining_user_budget_metric", CollectorKind::Gauge),
    ("litellm_user_max_budget_metric", CollectorKind::Gauge),
    (
        "litellm_user_budget_remaining_hours_metric",
        CollectorKind::Gauge,
    ),
    (
        "litellm_remaining_customer_budget_metric",
        CollectorKind::Gauge,
    ),
    ("litellm_customer_max_budget_metric", CollectorKind::Gauge),
    (
        "litellm_customer_budget_remaining_hours_metric",
        CollectorKind::Gauge,
    ),
    (
        "litellm_remaining_api_key_requests_for_model",
        CollectorKind::Gauge,
    ),
    (
        "litellm_remaining_api_key_tokens_for_model",
        CollectorKind::Gauge,
    ),
    (
        "litellm_api_key_rate_limit_allowed_metric",
        CollectorKind::Gauge,
    ),
    (
        "litellm_api_key_rate_limit_used_metric",
        CollectorKind::Gauge,
    ),
    (
        "litellm_team_rate_limit_allowed_metric",
        CollectorKind::Gauge,
    ),
    ("litellm_team_rate_limit_used_metric", CollectorKind::Gauge),
    ("litellm_remaining_requests_metric", CollectorKind::Gauge),
    ("litellm_remaining_tokens_metric", CollectorKind::Gauge),
    ("litellm_overhead_latency_metric", CollectorKind::Histogram),
    (
        "litellm_overhead_with_guardrails_latency_metric",
        CollectorKind::Histogram,
    ),
    (
        "litellm_request_queue_time_seconds",
        CollectorKind::Histogram,
    ),
    (
        "litellm_guardrail_latency_seconds",
        CollectorKind::Histogram,
    ),
    ("litellm_guardrail_errors", CollectorKind::Counter),
    ("litellm_guardrail_requests", CollectorKind::Counter),
    (
        "litellm_provider_remaining_budget_metric",
        CollectorKind::Gauge,
    ),
    ("litellm_deployment_state", CollectorKind::Gauge),
    ("litellm_deployment_tpm_limit", CollectorKind::Gauge),
    ("litellm_deployment_rpm_limit", CollectorKind::Gauge),
    ("litellm_deployment_cooled_down", CollectorKind::Counter),
    (
        "litellm_deployment_success_responses",
        CollectorKind::Counter,
    ),
    (
        "litellm_deployment_failure_responses",
        CollectorKind::Counter,
    ),
    ("litellm_deployment_total_requests", CollectorKind::Counter),
    (
        "litellm_deployment_latency_per_output_token",
        CollectorKind::Histogram,
    ),
    (
        "litellm_deployment_successful_fallbacks",
        CollectorKind::Counter,
    ),
    (
        "litellm_deployment_failed_fallbacks",
        CollectorKind::Counter,
    ),
    (
        "litellm_callback_logging_failures_metric",
        CollectorKind::Counter,
    ),
    (
        "litellm_llm_api_failed_requests_metric",
        CollectorKind::Counter,
    ),
    ("litellm_requests_metric", CollectorKind::Counter),
    ("litellm_zero_cost_requests", CollectorKind::Counter),
    ("litellm_spend_capture_rate", CollectorKind::Gauge),
    ("litellm_cache_hits_metric", CollectorKind::Counter),
    ("litellm_cache_misses_metric", CollectorKind::Counter),
    ("litellm_cached_tokens_metric", CollectorKind::Counter),
    (
        "litellm_provider_cache_read_input_tokens_metric",
        CollectorKind::Counter,
    ),
    (
        "litellm_provider_cache_creation_input_tokens_metric",
        CollectorKind::Counter,
    ),
    ("litellm_total_users", CollectorKind::Gauge),
    ("litellm_active_users", CollectorKind::Gauge),
    ("litellm_teams_count", CollectorKind::Gauge),
    ("litellm_managed_batch_created", CollectorKind::Counter),
    ("litellm_managed_file_size_bytes", CollectorKind::Gauge),
    (
        "litellm_managed_batch_duration_seconds",
        CollectorKind::Histogram,
    ),
    ("litellm_managed_file_created", CollectorKind::Counter),
    ("litellm_managed_file_deleted", CollectorKind::Counter),
    ("litellm_check_batch_cost_jobs_polled", CollectorKind::Gauge),
    (
        "litellm_check_batch_cost_jobs_processed",
        CollectorKind::Counter,
    ),
    ("litellm_check_batch_cost_errors", CollectorKind::Counter),
    (
        "litellm_check_batch_cost_last_run_timestamp",
        CollectorKind::Gauge,
    ),
    ("litellm_mcp_tool_calls", CollectorKind::Counter),
    ("litellm_mcp_tool_call_spend_metric", CollectorKind::Counter),
];

#[test]
fn registers_the_81_collectors_in_litellm_order() {
    let (registry, _collectors) = registered(
        &PrometheusExporterSettings::default(),
        &default_label_settings(),
    );
    let descriptions = registry.descriptions();
    assert_eq!(descriptions.len(), 81);
    let actual: Vec<(&str, CollectorKind)> = descriptions
        .iter()
        .map(|description| (description.name.as_str(), description.kind))
        .collect();
    assert_eq!(actual, EXPECTED);
}
