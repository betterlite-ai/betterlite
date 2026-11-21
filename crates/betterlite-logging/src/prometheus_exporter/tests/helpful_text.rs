use betterlite_prometheus::collector_registry::CollectorRegistry;

use crate::prometheus_exporter::PrometheusExporterSettings;

use super::{default_label_settings, registered};

fn help(registry: &CollectorRegistry, name: &str) -> String {
    registry
        .get(name)
        .expect("the collector is registered")
        .description
        .help
        .clone()
}

#[test]
fn latency_help_keeps_the_concatenated_sentences() {
    let (registry, _collectors) = registered(
        &PrometheusExporterSettings::default(),
        &default_label_settings(),
    );
    assert_eq!(
        help(&registry, "litellm_request_total_latency_metric"),
        "End-to-end latency (seconds) for a request to LiteLLM Proxy Server, from the moment the request reached the proxy through the end of processing -- includes authentication, pre-call hooks, the LLM API call, and post-call processing"
    );
    assert_eq!(
        help(&registry, "litellm_request_queue_time_seconds"),
        "Time (seconds) from request arrival at the proxy to the start of pre-call processing -- includes authentication and any ASGI-level queueing"
    );
    assert_eq!(
        help(&registry, "litellm_overhead_with_guardrails_latency_metric"),
        "Total internal latency (seconds) added by LiteLLM, including pre/post-call guardrails (excludes the LLM API call)"
    );
    assert_eq!(
        help(&registry, "litellm_overhead_latency_metric"),
        "Latency overhead (milliseconds) added by LiteLLM processing"
    );
}

#[test]
fn budget_and_spend_help_keeps_litellms_wording() {
    let (registry, _collectors) = registered(
        &PrometheusExporterSettings::default(),
        &default_label_settings(),
    );
    assert_eq!(
        help(&registry, "litellm_team_budget_remaining_hours_metric"),
        "Remaining days for team budget to be reset"
    );
    assert_eq!(
        help(&registry, "litellm_org_budget_remaining_hours_metric"),
        "Remaining hours for org budget to be reset"
    );
    assert_eq!(
        help(&registry, "litellm_zero_cost_requests"),
        "Requests that carried usage but were logged at $0 on a model whose pricing entry has a non-zero rate, by reason (missing_pricing_key, pricing_not_applied, cost_calculation_error)"
    );
    assert_eq!(
        help(&registry, "litellm_spend_capture_rate"),
        "Share of the provider's bill LiteLLM captured as spend over the scheduled check's window (captured spend / provider bill), by api_provider; NaN when the last check produced no rate"
    );
}

#[test]
fn deployment_and_deprecated_help_keeps_the_typo() {
    let (registry, _collectors) = registered(
        &PrometheusExporterSettings::default(),
        &default_label_settings(),
    );
    assert_eq!(
        help(&registry, "litellm_deployment_failure_responses"),
        "LLM Deployment Analytics - Total number of failed LLM API calls for a specific LLM deploymeny. exception_status is the status of the exception from the llm api"
    );
    assert_eq!(
        help(&registry, "litellm_llm_api_failed_requests_metric"),
        "deprecated - use litellm_proxy_failed_requests_metric"
    );
    assert_eq!(
        help(&registry, "litellm_requests_metric"),
        "deprecated - use litellm_proxy_total_requests_metric. Total number of LLM calls to litellm - track total per API Key, team, user"
    );
}

#[test]
fn managed_and_batch_help_keeps_litellms_wording() {
    let (registry, _collectors) = registered(
        &PrometheusExporterSettings::default(),
        &default_label_settings(),
    );
    assert_eq!(
        help(&registry, "litellm_managed_file_deleted"),
        "Total number of managed file deletions (success or blocked)"
    );
    assert_eq!(
        help(&registry, "litellm_check_batch_cost_jobs_polled"),
        "Number of unprocessed batches found by the last CheckBatchCost poll"
    );
    assert_eq!(
        help(&registry, "litellm_check_batch_cost_last_run_timestamp"),
        "Unix timestamp of the last CheckBatchCost job run"
    );
    assert_eq!(
        help(&registry, "litellm_mcp_tool_calls"),
        "Total MCP tool calls, segmented by tool and server name"
    );
}
