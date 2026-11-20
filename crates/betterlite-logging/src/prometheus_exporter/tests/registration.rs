use std::convert::Infallible;

use betterlite_prometheus::buckets::{BATCH_DURATION_BUCKETS, LATENCY_BUCKETS, TimeBound};
use betterlite_prometheus::caller_identity_logic::validate_caller_identity;
use betterlite_prometheus::collector_factory::collector::MetricLike;
use betterlite_prometheus::collector_factory::factory::{CollectorFactory, is_metric_enabled};
use betterlite_prometheus::collector_registry::CollectorRegistry;
use betterlite_prometheus::collector_registry::metric::{
    Bucket, CollectorKind, PrometheusMetric, SeriesRemoval, prepare_buckets,
};

use crate::prometheus_helpers::bounded_series_tracker::{
    BoundedPrometheusSeriesTracker, ChildRemoval, RemovableMetric,
};
use crate::prometheus_label_schema::PrometheusLabelSettings;

use super::{
    Handle, LabelSchemaAdapter, PrometheusCollectors, PrometheusExporterError,
    PrometheusExporterSettings,
};

// prometheus_client Histogram.DEFAULT_BUCKETS, for the one histogram LiteLLM leaves without buckets
const CLIENT_DEFAULT_BUCKETS: [Bucket; 15] = [
    Bucket::Seconds(0.005),
    Bucket::Seconds(0.01),
    Bucket::Seconds(0.025),
    Bucket::Seconds(0.05),
    Bucket::Seconds(0.075),
    Bucket::Seconds(0.1),
    Bucket::Seconds(0.25),
    Bucket::Seconds(0.5),
    Bucket::Seconds(0.75),
    Bucket::Seconds(1.0),
    Bucket::Seconds(2.5),
    Bucket::Seconds(5.0),
    Bucket::Seconds(7.5),
    Bucket::Seconds(10.0),
    Bucket::Infinite,
];

// the labelnames argument of a collector: schema-resolved for its metric name, or the literal list
#[derive(Clone, Copy, Debug)]
enum Labels<'label> {
    Schema(&'label str),
    Fixed(&'label [&'label str]),
}

#[derive(Clone, Copy, Debug)]
enum BucketSet {
    Latency,
    BatchDuration,
    ClientDefault,
}

struct Builder<'builder, 'settings> {
    factory: &'builder mut CollectorFactory<LabelSchemaAdapter<'settings>>,
    registry: &'builder CollectorRegistry,
    latency_buckets: &'builder [Bucket],
}

impl<'builder, 'settings> Builder<'builder, 'settings> {
    fn resolve(&mut self, labels: Labels<'_>) -> Result<Vec<String>, PrometheusExporterError> {
        match labels {
            Labels::Schema(metric) => Ok(self.factory.get_labels_for_metric(metric)?),
            Labels::Fixed(names) => Ok(names.iter().map(|name| (*name).to_owned()).collect()),
        }
    }

    fn counter(
        &mut self,
        name: &str,
        help: &str,
        labels: Labels<'_>,
    ) -> Result<Handle, PrometheusExporterError> {
        let resolved = self.resolve(labels)?;
        let label_names: Vec<&str> = resolved.iter().map(String::as_str).collect();
        let handle = self
            .factory
            .create_metric_factory(|metric_name, labelnames| {
                PrometheusMetric::new(
                    CollectorKind::Counter,
                    metric_name,
                    help,
                    labelnames,
                    Vec::new(),
                )
            })(name, &label_names);
        self.store(handle)
    }

    fn gauge(
        &mut self,
        name: &str,
        help: &str,
        labels: Labels<'_>,
    ) -> Result<Handle, PrometheusExporterError> {
        let resolved = self.resolve(labels)?;
        let label_names: Vec<&str> = resolved.iter().map(String::as_str).collect();
        let handle = self
            .factory
            .create_metric_factory(|metric_name, labelnames| {
                PrometheusMetric::new(
                    CollectorKind::Gauge,
                    metric_name,
                    help,
                    labelnames,
                    Vec::new(),
                )
            })(name, &label_names);
        self.store(handle)
    }

    fn histogram(
        &mut self,
        name: &str,
        help: &str,
        labels: Labels<'_>,
        set: BucketSet,
    ) -> Result<Handle, PrometheusExporterError> {
        let resolved = self.resolve(labels)?;
        let label_names: Vec<&str> = resolved.iter().map(String::as_str).collect();
        let prepared = self.prepared_bounds(name, set)?;
        let handle = self
            .factory
            .create_metric_factory(|metric_name, labelnames| {
                PrometheusMetric::new(
                    CollectorKind::Histogram,
                    metric_name,
                    help,
                    labelnames,
                    prepared.clone(),
                )
            })(name, &label_names);
        self.store(handle)
    }

    // a disabled metric never reaches Histogram.__init__, so its buckets are never validated
    fn prepared_bounds(
        &self,
        name: &str,
        set: BucketSet,
    ) -> Result<Vec<Bucket>, PrometheusExporterError> {
        if !is_metric_enabled(
            &self.factory.enabled_metrics,
            &self.factory.exclude_metrics,
            name,
        ) {
            return Ok(Vec::new());
        }
        let bounds = match set {
            BucketSet::Latency => self.latency_buckets.to_vec(),
            BucketSet::BatchDuration => time_bounds(&BATCH_DURATION_BUCKETS),
            BucketSet::ClientDefault => CLIENT_DEFAULT_BUCKETS.to_vec(),
        };
        Ok(prepare_buckets(&bounds)?)
    }

    // the constructor registers the metric; a no-op one never reaches the registry and a label-filtered one registers without the excluded labels
    fn store(&self, handle: Handle) -> Result<Handle, PrometheusExporterError> {
        match &handle {
            MetricLike::NoOp(_) => {}
            MetricLike::Excluded(excluded) => self.registry.register(excluded.metric.clone())?,
            MetricLike::Real(metric) => self.registry.register(metric.clone())?,
        }
        Ok(handle)
    }
}

fn time_bounds(bounds: &[TimeBound]) -> Vec<Bucket> {
    bounds
        .iter()
        .map(|bound| match bound {
            TimeBound::Finite(duration) => Bucket::Seconds(duration.as_secs_f64()),
            TimeBound::Infinite => Bucket::Infinite,
        })
        .collect()
}

// PrometheusLogger.__init__ entry order: name, help text, labelnames and buckets as the constructor passes them
macro_rules! registered_collectors {
    ($builder:ident, $factory:ident, $settings:ident, $latency_buckets:ident) => {
        PrometheusCollectors {
            litellm_proxy_failed_requests_metric: $builder.counter(
                "litellm_proxy_failed_requests_metric",
                "Total number of failed responses from proxy - the client did not get a success response from litellm proxy",
                Labels::Schema("litellm_proxy_failed_requests_metric"),
            )?,
            litellm_proxy_total_requests_metric: $builder.counter(
                "litellm_proxy_total_requests_metric",
                "Total number of requests made to the proxy server - track number of client side requests",
                Labels::Schema("litellm_proxy_total_requests_metric"),
            )?,
            litellm_request_total_latency_metric: $builder.histogram(
                "litellm_request_total_latency_metric",
                "End-to-end latency (seconds) for a request to LiteLLM Proxy Server, from the moment the request reached the proxy through the end of processing -- includes authentication, pre-call hooks, the LLM API call, and post-call processing",
                Labels::Schema("litellm_request_total_latency_metric"),
                BucketSet::Latency,
            )?,
            litellm_llm_api_latency_metric: $builder.histogram(
                "litellm_llm_api_latency_metric",
                "Total latency (seconds) for a models LLM API call",
                Labels::Schema("litellm_llm_api_latency_metric"),
                BucketSet::Latency,
            )?,
            litellm_llm_api_time_to_first_token_metric: $builder.histogram(
                "litellm_llm_api_time_to_first_token_metric",
                "Time to first token for a models LLM API call",
                Labels::Schema("litellm_llm_api_time_to_first_token_metric"),
                BucketSet::Latency,
            )?,
            litellm_spend_metric: $builder.counter(
                "litellm_spend_metric",
                "Total spend on LLM requests",
                Labels::Schema("litellm_spend_metric"),
            )?,
            litellm_tokens_metric: $builder.counter(
                "litellm_total_tokens_metric",
                "Total number of input + output tokens from LLM requests",
                Labels::Schema("litellm_total_tokens_metric"),
            )?,
            litellm_input_tokens_metric: $builder.counter(
                "litellm_input_tokens_metric",
                "Total number of input tokens from LLM requests",
                Labels::Schema("litellm_input_tokens_metric"),
            )?,
            litellm_output_tokens_metric: $builder.counter(
                "litellm_output_tokens_metric",
                "Total number of output tokens from LLM requests",
                Labels::Schema("litellm_output_tokens_metric"),
            )?,
            litellm_input_cached_tokens_metric: $builder.counter(
                "litellm_input_cached_tokens_metric",
                "Provider-side cached input tokens (e.g. OpenAI prompt_tokens_details.cached_tokens, Anthropic cache_read_input_tokens)",
                Labels::Schema("litellm_input_cached_tokens_metric"),
            )?,
            litellm_input_cache_creation_tokens_metric: $builder.counter(
                "litellm_input_cache_creation_tokens_metric",
                "Provider-side input tokens written to prompt cache (e.g. Anthropic cache_creation_input_tokens)",
                Labels::Schema("litellm_input_cache_creation_tokens_metric"),
            )?,
            litellm_input_audio_tokens_metric: $builder.counter(
                "litellm_input_audio_tokens_metric",
                "Audio input tokens reported in prompt_tokens_details.audio_tokens",
                Labels::Schema("litellm_input_audio_tokens_metric"),
            )?,
            litellm_output_reasoning_tokens_metric: $builder.counter(
                "litellm_output_reasoning_tokens_metric",
                "Reasoning tokens reported in completion_tokens_details.reasoning_tokens",
                Labels::Schema("litellm_output_reasoning_tokens_metric"),
            )?,
            litellm_output_audio_tokens_metric: $builder.counter(
                "litellm_output_audio_tokens_metric",
                "Audio output tokens reported in completion_tokens_details.audio_tokens",
                Labels::Schema("litellm_output_audio_tokens_metric"),
            )?,
            litellm_video_duration_seconds_metric: $builder.counter(
                "litellm_video_duration_seconds_metric",
                "Seconds of video generated, from usage.duration_seconds on video generation calls",
                Labels::Schema("litellm_video_duration_seconds_metric"),
            )?,
            litellm_images_generated_metric: $builder.counter(
                "litellm_images_generated_metric",
                "Number of images generated, from the image generation response",
                Labels::Schema("litellm_images_generated_metric"),
            )?,
            litellm_remaining_team_budget_metric: $builder.gauge(
                "litellm_remaining_team_budget_metric",
                "Remaining budget for team",
                Labels::Schema("litellm_remaining_team_budget_metric"),
            )?,
            litellm_team_max_budget_metric: $builder.gauge(
                "litellm_team_max_budget_metric",
                "Maximum budget set for team",
                Labels::Schema("litellm_team_max_budget_metric"),
            )?,
            litellm_team_budget_remaining_hours_metric: $builder.gauge(
                "litellm_team_budget_remaining_hours_metric",
                "Remaining days for team budget to be reset",
                Labels::Schema("litellm_team_budget_remaining_hours_metric"),
            )?,
            litellm_team_members_metric: $builder.gauge(
                "litellm_team_members_metric",
                "Number of members in a team",
                Labels::Schema("litellm_team_members_metric"),
            )?,
            litellm_remaining_org_budget_metric: $builder.gauge(
                "litellm_remaining_org_budget_metric",
                "Remaining budget for org",
                Labels::Schema("litellm_remaining_org_budget_metric"),
            )?,
            litellm_org_max_budget_metric: $builder.gauge(
                "litellm_org_max_budget_metric",
                "Maximum budget set for org",
                Labels::Schema("litellm_org_max_budget_metric"),
            )?,
            litellm_org_budget_remaining_hours_metric: $builder.gauge(
                "litellm_org_budget_remaining_hours_metric",
                "Remaining hours for org budget to be reset",
                Labels::Schema("litellm_org_budget_remaining_hours_metric"),
            )?,
            litellm_remaining_api_key_budget_metric: $builder.gauge(
                "litellm_remaining_api_key_budget_metric",
                "Remaining budget for api key",
                Labels::Schema("litellm_remaining_api_key_budget_metric"),
            )?,
            litellm_api_key_max_budget_metric: $builder.gauge(
                "litellm_api_key_max_budget_metric",
                "Maximum budget set for api key",
                Labels::Schema("litellm_api_key_max_budget_metric"),
            )?,
            litellm_api_key_budget_remaining_hours_metric: $builder.gauge(
                "litellm_api_key_budget_remaining_hours_metric",
                "Remaining hours for api key budget to be reset",
                Labels::Schema("litellm_api_key_budget_remaining_hours_metric"),
            )?,
            litellm_remaining_user_budget_metric: $builder.gauge(
                "litellm_remaining_user_budget_metric",
                "Remaining budget for user",
                Labels::Schema("litellm_remaining_user_budget_metric"),
            )?,
            litellm_user_max_budget_metric: $builder.gauge(
                "litellm_user_max_budget_metric",
                "Maximum budget set for user",
                Labels::Schema("litellm_user_max_budget_metric"),
            )?,
            litellm_user_budget_remaining_hours_metric: $builder.gauge(
                "litellm_user_budget_remaining_hours_metric",
                "Remaining hours for user budget to be reset",
                Labels::Schema("litellm_user_budget_remaining_hours_metric"),
            )?,
            litellm_remaining_customer_budget_metric: $builder.gauge(
                "litellm_remaining_customer_budget_metric",
                "Remaining budget for customer (end user)",
                Labels::Schema("litellm_remaining_customer_budget_metric"),
            )?,
            litellm_customer_max_budget_metric: $builder.gauge(
                "litellm_customer_max_budget_metric",
                "Maximum budget set for customer (end user)",
                Labels::Schema("litellm_customer_max_budget_metric"),
            )?,
            litellm_customer_budget_remaining_hours_metric: $builder.gauge(
                "litellm_customer_budget_remaining_hours_metric",
                "Remaining hours for customer (end user) budget to be reset",
                Labels::Schema("litellm_customer_budget_remaining_hours_metric"),
            )?,
            litellm_remaining_api_key_requests_for_model: $builder.gauge(
                "litellm_remaining_api_key_requests_for_model",
                "Remaining Requests API Key can make for model (model based rpm limit on key)",
                Labels::Schema("litellm_remaining_api_key_requests_for_model"),
            )?,
            litellm_remaining_api_key_tokens_for_model: $builder.gauge(
                "litellm_remaining_api_key_tokens_for_model",
                "Remaining Tokens API Key can make for model (model based tpm limit on key)",
                Labels::Schema("litellm_remaining_api_key_tokens_for_model"),
            )?,
            litellm_api_key_rate_limit_allowed_metric: $builder.gauge(
                "litellm_api_key_rate_limit_allowed_metric",
                "Configured rate limit for the API Key in the current window (rpm_limit / tpm_limit), by rate_limit_type",
                Labels::Schema("litellm_api_key_rate_limit_allowed_metric"),
            )?,
            litellm_api_key_rate_limit_used_metric: $builder.gauge(
                "litellm_api_key_rate_limit_used_metric",
                "Requests or tokens the API Key has consumed in the current rate limit window, by rate_limit_type",
                Labels::Schema("litellm_api_key_rate_limit_used_metric"),
            )?,
            litellm_team_rate_limit_allowed_metric: $builder.gauge(
                "litellm_team_rate_limit_allowed_metric",
                "Configured rate limit for the Team in the current window (team rpm_limit / tpm_limit), by rate_limit_type",
                Labels::Schema("litellm_team_rate_limit_allowed_metric"),
            )?,
            litellm_team_rate_limit_used_metric: $builder.gauge(
                "litellm_team_rate_limit_used_metric",
                "Requests or tokens the Team has consumed in the current rate limit window, by rate_limit_type",
                Labels::Schema("litellm_team_rate_limit_used_metric"),
            )?,
            litellm_remaining_requests_metric: $builder.gauge(
                "litellm_remaining_requests_metric",
                "LLM Deployment Analytics - remaining requests for model, returned from LLM API Provider",
                Labels::Schema("litellm_remaining_requests_metric"),
            )?,
            litellm_remaining_tokens_metric: $builder.gauge(
                "litellm_remaining_tokens_metric",
                "remaining tokens for model, returned from LLM API Provider",
                Labels::Schema("litellm_remaining_tokens_metric"),
            )?,
            litellm_overhead_latency_metric: $builder.histogram(
                "litellm_overhead_latency_metric",
                "Latency overhead (milliseconds) added by LiteLLM processing",
                Labels::Schema("litellm_overhead_latency_metric"),
                BucketSet::Latency,
            )?,
            litellm_overhead_with_guardrails_latency_metric: $builder.histogram(
                "litellm_overhead_with_guardrails_latency_metric",
                "Total internal latency (seconds) added by LiteLLM, including pre/post-call guardrails (excludes the LLM API call)",
                Labels::Schema("litellm_overhead_with_guardrails_latency_metric"),
                BucketSet::Latency,
            )?,
            litellm_request_queue_time_metric: $builder.histogram(
                "litellm_request_queue_time_seconds",
                "Time (seconds) from request arrival at the proxy to the start of pre-call processing -- includes authentication and any ASGI-level queueing",
                Labels::Schema("litellm_request_queue_time_seconds"),
                BucketSet::Latency,
            )?,
            litellm_guardrail_latency_metric: $builder.histogram(
                "litellm_guardrail_latency_seconds",
                "Latency (seconds) for guardrail execution",
                Labels::Fixed(&["guardrail_name", "status", "error_type", "hook_type"]),
                BucketSet::Latency,
            )?,
            litellm_guardrail_errors_total: $builder.counter(
                "litellm_guardrail_errors_total",
                "Total number of errors encountered during guardrail execution",
                Labels::Fixed(&["guardrail_name", "error_type", "hook_type"]),
            )?,
            litellm_guardrail_requests_total: $builder.counter(
                "litellm_guardrail_requests_total",
                "Total number of guardrail invocations",
                Labels::Fixed(&["guardrail_name", "status", "hook_type"]),
            )?,
            litellm_provider_remaining_budget_metric: $builder.gauge(
                "litellm_provider_remaining_budget_metric",
                "Remaining budget for provider - used when you set provider budget limits",
                Labels::Fixed(&["api_provider"]),
            )?,
            litellm_deployment_state: $builder.gauge(
                "litellm_deployment_state",
                "LLM Deployment Analytics - The state of the deployment: 0 = healthy, 1 = partial outage, 2 = complete outage",
                Labels::Schema("litellm_deployment_state"),
            )?,
            litellm_deployment_tpm_limit: $builder.gauge(
                "litellm_deployment_tpm_limit",
                "Deployment TPM limit found in config",
                Labels::Schema("litellm_deployment_tpm_limit"),
            )?,
            litellm_deployment_rpm_limit: $builder.gauge(
                "litellm_deployment_rpm_limit",
                "Deployment RPM limit found in config",
                Labels::Schema("litellm_deployment_rpm_limit"),
            )?,
            litellm_deployment_cooled_down: $builder.counter(
                "litellm_deployment_cooled_down",
                "LLM Deployment Analytics - Number of times a deployment has been cooled down by LiteLLM load balancing logic. exception_status is the status of the exception that caused the deployment to be cooled down",
                Labels::Schema("litellm_deployment_cooled_down"),
            )?,
            litellm_deployment_success_responses: $builder.counter(
                "litellm_deployment_success_responses",
                "LLM Deployment Analytics - Total number of successful LLM API calls via litellm",
                Labels::Schema("litellm_deployment_success_responses"),
            )?,
            litellm_deployment_failure_responses: $builder.counter(
                "litellm_deployment_failure_responses",
                "LLM Deployment Analytics - Total number of failed LLM API calls for a specific LLM deploymeny. exception_status is the status of the exception from the llm api",
                Labels::Schema("litellm_deployment_failure_responses"),
            )?,
            litellm_deployment_total_requests: $builder.counter(
                "litellm_deployment_total_requests",
                "LLM Deployment Analytics - Total number of LLM API calls via litellm - success + failure",
                Labels::Schema("litellm_deployment_total_requests"),
            )?,
            litellm_deployment_latency_per_output_token: $builder.histogram(
                "litellm_deployment_latency_per_output_token",
                "LLM Deployment Analytics - Latency per output token",
                Labels::Schema("litellm_deployment_latency_per_output_token"),
                BucketSet::ClientDefault,
            )?,
            litellm_deployment_successful_fallbacks: $builder.counter(
                "litellm_deployment_successful_fallbacks",
                "LLM Deployment Analytics - Number of successful fallback requests from primary model -> fallback model",
                Labels::Schema("litellm_deployment_successful_fallbacks"),
            )?,
            litellm_deployment_failed_fallbacks: $builder.counter(
                "litellm_deployment_failed_fallbacks",
                "LLM Deployment Analytics - Number of failed fallback requests from primary model -> fallback model",
                Labels::Schema("litellm_deployment_failed_fallbacks"),
            )?,
            litellm_callback_logging_failures_metric: $builder.counter(
                "litellm_callback_logging_failures_metric",
                "Total number of failures when emitting logs to callbacks (e.g. s3_v2, langfuse, etc)",
                Labels::Fixed(&["callback_name"]),
            )?,
            litellm_llm_api_failed_requests_metric: $builder.counter(
                "litellm_llm_api_failed_requests_metric",
                "deprecated - use litellm_proxy_failed_requests_metric",
                Labels::Schema("litellm_llm_api_failed_requests_metric"),
            )?,
            litellm_requests_metric: $builder.counter(
                "litellm_requests_metric",
                "deprecated - use litellm_proxy_total_requests_metric. Total number of LLM calls to litellm - track total per API Key, team, user",
                Labels::Schema("litellm_requests_metric"),
            )?,
            litellm_zero_cost_requests_total: $builder.counter(
                "litellm_zero_cost_requests_total",
                "Requests that carried usage but were logged at $0 on a model whose pricing entry has a non-zero rate, by reason (missing_pricing_key, pricing_not_applied, cost_calculation_error)",
                Labels::Schema("litellm_zero_cost_requests_total"),
            )?,
            litellm_spend_capture_rate: $builder.gauge(
                "litellm_spend_capture_rate",
                "Share of the provider's bill LiteLLM captured as spend over the scheduled check's window (captured spend / provider bill), by api_provider; NaN when the last check produced no rate",
                Labels::Schema("litellm_spend_capture_rate"),
            )?,
            litellm_cache_hits_metric: $builder.counter(
                "litellm_cache_hits_metric",
                "Total number of LiteLLM cache hits",
                Labels::Schema("litellm_cache_hits_metric"),
            )?,
            litellm_cache_misses_metric: $builder.counter(
                "litellm_cache_misses_metric",
                "Total number of LiteLLM cache misses",
                Labels::Schema("litellm_cache_misses_metric"),
            )?,
            litellm_cached_tokens_metric: $builder.counter(
                "litellm_cached_tokens_metric",
                "Total tokens served from LiteLLM cache",
                Labels::Schema("litellm_cached_tokens_metric"),
            )?,
            litellm_provider_cache_read_input_tokens_metric: $builder.counter(
                "litellm_provider_cache_read_input_tokens_metric",
                "Total prompt/input tokens read from provider prompt cache (e.g. OpenAI/Anthropic/Gemini/Bedrock)",
                Labels::Schema("litellm_provider_cache_read_input_tokens_metric"),
            )?,
            litellm_provider_cache_creation_input_tokens_metric: $builder.counter(
                "litellm_provider_cache_creation_input_tokens_metric",
                "Total prompt/input tokens written to provider prompt cache (e.g. Anthropic/Bedrock)",
                Labels::Schema("litellm_provider_cache_creation_input_tokens_metric"),
            )?,
            litellm_total_users_metric: $builder.gauge(
                "litellm_total_users",
                "Total number of users in LiteLLM",
                Labels::Fixed(&[]),
            )?,
            litellm_active_users_metric: $builder.gauge(
                "litellm_active_users",
                "Number of billable users in LiteLLM (excludes SCIM-deactivated users)",
                Labels::Fixed(&[]),
            )?,
            litellm_teams_count_metric: $builder.gauge(
                "litellm_teams_count",
                "Total number of teams in LiteLLM",
                Labels::Fixed(&[]),
            )?,
            litellm_managed_batch_created_total: $builder.counter(
                "litellm_managed_batch_created_total",
                "Total number of managed batches created",
                Labels::Fixed(&["model", "api_provider", "user", "user_email", "api_key_alias"]),
            )?,
            litellm_managed_file_size_bytes: $builder.gauge(
                "litellm_managed_file_size_bytes",
                "Size of the most recent managed batch file in bytes (last-seen value per label combination)",
                Labels::Fixed(&["purpose", "file_type", "model", "api_provider", "user"]),
            )?,
            litellm_managed_batch_duration_seconds: $builder.histogram(
                "litellm_managed_batch_duration_seconds",
                "Duration of completed managed batches in seconds (completed_at - created_at)",
                Labels::Fixed(&["model", "api_provider"]),
                BucketSet::BatchDuration,
            )?,
            litellm_managed_file_created_total: $builder.counter(
                "litellm_managed_file_created_total",
                "Total number of managed files created",
                Labels::Fixed(&["model", "api_provider", "user", "user_email", "api_key_alias"]),
            )?,
            litellm_managed_file_deleted_total: $builder.counter(
                "litellm_managed_file_deleted_total",
                "Total number of managed file deletions (success or blocked)",
                Labels::Fixed(&["result"]),
            )?,
            litellm_check_batch_cost_jobs_polled: $builder.gauge(
                "litellm_check_batch_cost_jobs_polled",
                "Number of unprocessed batches found by the last CheckBatchCost poll",
                Labels::Fixed(&[]),
            )?,
            litellm_check_batch_cost_jobs_processed_total: $builder.counter(
                "litellm_check_batch_cost_jobs_processed_total",
                "Total number of batches successfully cost-tracked by CheckBatchCost",
                Labels::Fixed(&["model", "api_provider"]),
            )?,
            litellm_check_batch_cost_errors_total: $builder.counter(
                "litellm_check_batch_cost_errors_total",
                "Total number of errors in CheckBatchCost by error type",
                Labels::Fixed(&["error_type"]),
            )?,
            litellm_check_batch_cost_last_run_timestamp: $builder.gauge(
                "litellm_check_batch_cost_last_run_timestamp",
                "Unix timestamp of the last CheckBatchCost job run",
                Labels::Fixed(&[]),
            )?,
            litellm_mcp_tool_calls_total: $builder.counter(
                "litellm_mcp_tool_calls_total",
                "Total MCP tool calls, segmented by tool and server name",
                Labels::Schema("litellm_mcp_tool_calls_total"),
            )?,
            litellm_mcp_tool_call_spend_metric: $builder.counter(
                "litellm_mcp_tool_call_spend_metric",
                "Total spend on MCP tool calls, segmented by tool and server name",
                Labels::Schema("litellm_mcp_tool_call_spend_metric"),
            )?,
            label_filters: $factory.label_filters.clone(),
            exclude_metrics: $factory.exclude_metrics.clone(),
            exclude_labels: $factory.exclude_labels.clone(),
            cached_labels: $factory.cached_labels.clone(),
            latency_buckets: $latency_buckets,
            emit_input_sequence_length_label: $settings.prometheus_emit_input_sequence_length_label,
            bounded_series_tracker: BoundedPrometheusSeriesTracker::default(),
        }
    };
}

impl PrometheusCollectors {
    pub fn register(
        settings: &PrometheusExporterSettings,
        label_settings: &PrometheusLabelSettings,
        registry: &CollectorRegistry,
    ) -> Result<Self, PrometheusExporterError> {
        Self::construct(settings, label_settings, registry)
            .inspect_err(|error| tracing::debug!("Got exception on init prometheus client {error}"))
    }

    fn construct(
        settings: &PrometheusExporterSettings,
        label_settings: &PrometheusLabelSettings,
        registry: &CollectorRegistry,
    ) -> Result<Self, PrometheusExporterError> {
        validate_caller_identity(Some(label_settings.caller_identity.as_str()))?;
        let mut factory = CollectorFactory::new(LabelSchemaAdapter {
            settings: label_settings,
        });
        factory.parse_prometheus_config(&settings.prometheus_metrics_config)?;
        factory.parse_exclude_config(
            &settings.prometheus_exclude_metrics,
            &settings.prometheus_exclude_labels,
            &label_settings.custom_metadata_labels,
            &label_settings.custom_tags,
        )?;
        let latency_buckets = match &settings.prometheus_latency_buckets {
            Some(custom) => custom
                .iter()
                .map(|seconds| Bucket::Seconds(*seconds))
                .collect(),
            None => time_bounds(&LATENCY_BUCKETS),
        };
        let mut builder = Builder {
            factory: &mut factory,
            registry,
            latency_buckets: &latency_buckets,
        };
        let collectors = registered_collectors!(builder, factory, settings, latency_buckets);
        Ok(collectors)
    }
}
// _ExcludedLabelMetric has no remove in LiteLLM, and a no-op metric has no series to remove
impl RemovableMetric for MetricLike<PrometheusMetric> {
    type Error = Infallible;

    fn remove_child(&self, label_values: &[Option<String>]) -> Result<ChildRemoval, Self::Error> {
        match self {
            Self::NoOp(_) => Ok(ChildRemoval::Unsupported),
            Self::Excluded(_) => Ok(ChildRemoval::Unsupported),
            Self::Real(metric) => Ok(match metric.remove_series(label_values) {
                SeriesRemoval::Removed => ChildRemoval::Removed,
                SeriesRemoval::Absent => ChildRemoval::Absent,
                SeriesRemoval::Unsupported => ChildRemoval::Unsupported,
            }),
        }
    }
}
