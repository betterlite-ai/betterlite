use std::collections::BTreeMap;

use betterlite_prometheus::caller_identity_logic::CallerIdentityError;
use betterlite_prometheus::collector_factory::collector::MetricLike;
use betterlite_prometheus::collector_factory::config::{MetricSet, PrometheusMetricsConfig};
use betterlite_prometheus::collector_factory::error::CollectorFactoryError;
use betterlite_prometheus::collector_factory::factory::LabelSchema;
use betterlite_prometheus::collector_registry::CollectorRegistryError;
use betterlite_prometheus::collector_registry::metric::{Bucket, BucketError, PrometheusMetric};

use crate::prometheus_helpers::bounded_series_tracker::BoundedPrometheusSeriesTracker;
use crate::prometheus_label_schema::{LabelSchemaError, PrometheusLabelSettings, get_labels};

#[path = "prometheus_exporter/tests/registration.rs"]
mod registration;
mod tests;

pub type Handle = MetricLike<PrometheusMetric>;

// the litellm_settings the constructor reads; B8655 fills the lists it parses elsewhere
#[derive(Clone, Debug, Default)]
pub struct PrometheusExporterSettings {
    pub prometheus_metrics_config: Vec<PrometheusMetricsConfig>,
    pub prometheus_exclude_metrics: Vec<String>,
    pub prometheus_exclude_labels: Vec<String>,
    pub prometheus_latency_buckets: Option<Vec<f64>>,
    pub prometheus_emit_input_sequence_length_label: bool, // ast-grep-ignore: no-long-names
}

#[derive(Debug, thiserror::Error)]
pub enum PrometheusExporterError {
    #[error(transparent)]
    CallerIdentity(#[from] CallerIdentityError),
    #[error(transparent)]
    Factory(#[from] CollectorFactoryError<LabelSchemaError>),
    #[error(transparent)]
    Registry(#[from] CollectorRegistryError),
    #[error(transparent)]
    Buckets(#[from] BucketError),
}

struct LabelSchemaAdapter<'settings> {
    settings: &'settings PrometheusLabelSettings,
}

impl LabelSchema for LabelSchemaAdapter<'_> {
    type Error = LabelSchemaError;

    fn labels(&self, metric: &str) -> Result<Vec<String>, Self::Error> {
        get_labels(metric, self.settings)
    }

    fn caller_identity_mode(&self) -> &str {
        &self.settings.caller_identity
    }
}

// one field per PrometheusLogger attribute; every emitter task reads them by the LiteLLM name
#[derive(Debug)]
pub struct PrometheusCollectors {
    pub litellm_proxy_failed_requests_metric: Handle, // ast-grep-ignore: no-long-names
    pub litellm_proxy_total_requests_metric: Handle,  // ast-grep-ignore: no-long-names
    pub litellm_request_total_latency_metric: Handle, // ast-grep-ignore: no-long-names
    pub litellm_llm_api_latency_metric: Handle,
    pub litellm_llm_api_time_to_first_token_metric: Handle, // ast-grep-ignore: no-long-names
    pub litellm_spend_metric: Handle,
    pub litellm_tokens_metric: Handle,
    pub litellm_input_tokens_metric: Handle,
    pub litellm_output_tokens_metric: Handle,
    pub litellm_input_cached_tokens_metric: Handle, // ast-grep-ignore: no-long-names
    pub litellm_input_cache_creation_tokens_metric: Handle, // ast-grep-ignore: no-long-names
    pub litellm_input_audio_tokens_metric: Handle,  // ast-grep-ignore: no-long-names
    pub litellm_output_reasoning_tokens_metric: Handle, // ast-grep-ignore: no-long-names
    pub litellm_output_audio_tokens_metric: Handle, // ast-grep-ignore: no-long-names
    pub litellm_video_duration_seconds_metric: Handle, // ast-grep-ignore: no-long-names
    pub litellm_images_generated_metric: Handle,    // ast-grep-ignore: no-long-names
    pub litellm_remaining_team_budget_metric: Handle, // ast-grep-ignore: no-long-names
    pub litellm_team_max_budget_metric: Handle,
    pub litellm_team_budget_remaining_hours_metric: Handle, // ast-grep-ignore: no-long-names
    pub litellm_team_members_metric: Handle,
    pub litellm_remaining_org_budget_metric: Handle, // ast-grep-ignore: no-long-names
    pub litellm_org_max_budget_metric: Handle,
    pub litellm_org_budget_remaining_hours_metric: Handle, // ast-grep-ignore: no-long-names
    pub litellm_remaining_api_key_budget_metric: Handle,   // ast-grep-ignore: no-long-names
    pub litellm_api_key_max_budget_metric: Handle,         // ast-grep-ignore: no-long-names
    pub litellm_api_key_budget_remaining_hours_metric: Handle, // ast-grep-ignore: no-long-names
    pub litellm_remaining_user_budget_metric: Handle,      // ast-grep-ignore: no-long-names
    pub litellm_user_max_budget_metric: Handle,
    pub litellm_user_budget_remaining_hours_metric: Handle, // ast-grep-ignore: no-long-names
    pub litellm_remaining_customer_budget_metric: Handle,   // ast-grep-ignore: no-long-names
    pub litellm_customer_max_budget_metric: Handle,         // ast-grep-ignore: no-long-names
    pub litellm_customer_budget_remaining_hours_metric: Handle, // ast-grep-ignore: no-long-names
    pub litellm_remaining_api_key_requests_for_model: Handle, // ast-grep-ignore: no-long-names
    pub litellm_remaining_api_key_tokens_for_model: Handle, // ast-grep-ignore: no-long-names
    pub litellm_api_key_rate_limit_allowed_metric: Handle,  // ast-grep-ignore: no-long-names
    pub litellm_api_key_rate_limit_used_metric: Handle,     // ast-grep-ignore: no-long-names
    pub litellm_team_rate_limit_allowed_metric: Handle,     // ast-grep-ignore: no-long-names
    pub litellm_team_rate_limit_used_metric: Handle,        // ast-grep-ignore: no-long-names
    pub litellm_remaining_requests_metric: Handle,          // ast-grep-ignore: no-long-names
    pub litellm_remaining_tokens_metric: Handle,            // ast-grep-ignore: no-long-names
    pub litellm_overhead_latency_metric: Handle,            // ast-grep-ignore: no-long-names
    pub litellm_overhead_with_guardrails_latency_metric: Handle, // ast-grep-ignore: no-long-names
    pub litellm_request_queue_time_metric: Handle,          // ast-grep-ignore: no-long-names
    pub litellm_guardrail_latency_metric: Handle,           // ast-grep-ignore: no-long-names
    pub litellm_guardrail_errors_total: Handle,
    pub litellm_guardrail_requests_total: Handle, // ast-grep-ignore: no-long-names
    pub litellm_provider_remaining_budget_metric: Handle, // ast-grep-ignore: no-long-names
    pub litellm_deployment_state: Handle,
    pub litellm_deployment_tpm_limit: Handle,
    pub litellm_deployment_rpm_limit: Handle,
    pub litellm_deployment_cooled_down: Handle,
    pub litellm_deployment_success_responses: Handle, // ast-grep-ignore: no-long-names
    pub litellm_deployment_failure_responses: Handle, // ast-grep-ignore: no-long-names
    pub litellm_deployment_total_requests: Handle,    // ast-grep-ignore: no-long-names
    pub litellm_deployment_latency_per_output_token: Handle, // ast-grep-ignore: no-long-names
    pub litellm_deployment_successful_fallbacks: Handle, // ast-grep-ignore: no-long-names
    pub litellm_deployment_failed_fallbacks: Handle,  // ast-grep-ignore: no-long-names
    pub litellm_callback_logging_failures_metric: Handle, // ast-grep-ignore: no-long-names
    pub litellm_llm_api_failed_requests_metric: Handle, // ast-grep-ignore: no-long-names
    pub litellm_requests_metric: Handle,
    pub litellm_zero_cost_requests_total: Handle, // ast-grep-ignore: no-long-names
    pub litellm_spend_capture_rate: Handle,
    pub litellm_cache_hits_metric: Handle,
    pub litellm_cache_misses_metric: Handle,
    pub litellm_cached_tokens_metric: Handle,
    pub litellm_provider_cache_read_input_tokens_metric: Handle, // ast-grep-ignore: no-long-names
    pub litellm_provider_cache_creation_input_tokens_metric: Handle, // ast-grep-ignore: no-long-names
    pub litellm_total_users_metric: Handle,
    pub litellm_active_users_metric: Handle,
    pub litellm_teams_count_metric: Handle,
    pub litellm_managed_batch_created_total: Handle, // ast-grep-ignore: no-long-names
    pub litellm_managed_file_size_bytes: Handle,     // ast-grep-ignore: no-long-names
    pub litellm_managed_batch_duration_seconds: Handle, // ast-grep-ignore: no-long-names
    pub litellm_managed_file_created_total: Handle,  // ast-grep-ignore: no-long-names
    pub litellm_managed_file_deleted_total: Handle,  // ast-grep-ignore: no-long-names
    pub litellm_check_batch_cost_jobs_polled: Handle, // ast-grep-ignore: no-long-names
    pub litellm_check_batch_cost_jobs_processed_total: Handle, // ast-grep-ignore: no-long-names
    pub litellm_check_batch_cost_errors_total: Handle, // ast-grep-ignore: no-long-names
    pub litellm_check_batch_cost_last_run_timestamp: Handle, // ast-grep-ignore: no-long-names
    pub litellm_mcp_tool_calls_total: Handle,
    pub litellm_mcp_tool_call_spend_metric: Handle, // ast-grep-ignore: no-long-names
    pub label_filters: BTreeMap<String, Vec<String>>,
    pub exclude_metrics: MetricSet,
    pub exclude_labels: MetricSet,
    pub cached_labels: BTreeMap<String, Vec<String>>,
    pub latency_buckets: Vec<Bucket>,
    pub emit_input_sequence_length_label: bool, // ast-grep-ignore: no-long-names
    pub bounded_series_tracker: BoundedPrometheusSeriesTracker,
}
