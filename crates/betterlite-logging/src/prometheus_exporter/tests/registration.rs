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
    Empty,
    Latency,
    BatchDuration,
    ClientDefault,
}

// one collector call: the kind, the Prometheus name, the help text, the label source and the bucket set
#[derive(Clone, Copy, Debug)]
struct CollectorSpec<'spec> {
    kind: CollectorKind,
    name: &'spec str,
    help: &'spec str,
    labels: Labels<'spec>,
    set: BucketSet,
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

    fn collector(&mut self, spec: CollectorSpec<'_>) -> Result<Handle, PrometheusExporterError> {
        let resolved = self.resolve(spec.labels)?;
        let label_names: Vec<&str> = resolved.iter().map(String::as_str).collect();
        let prepared = self.prepared_bounds(spec.name, spec.set)?;
        let handle = self
            .factory
            .create_metric_factory(|metric_name, labelnames| {
                PrometheusMetric::new(
                    spec.kind,
                    metric_name,
                    spec.help,
                    labelnames,
                    prepared.clone(),
                )
            })(spec.name, label_names.as_slice());
        self.store(handle)
    }

    // a disabled metric never reaches Histogram.__init__, so its buckets are never validated
    fn prepared_bounds(
        &self,
        name: &str,
        set: BucketSet,
    ) -> Result<Vec<Bucket>, PrometheusExporterError> {
        let bounds = match set {
            BucketSet::Empty => return Ok(Vec::new()),
            BucketSet::Latency => self.latency_buckets.to_vec(),
            BucketSet::BatchDuration => time_bounds(&BATCH_DURATION_BUCKETS),
            BucketSet::ClientDefault => CLIENT_DEFAULT_BUCKETS.to_vec(),
        };
        if !is_metric_enabled(
            &self.factory.enabled_metrics,
            &self.factory.exclude_metrics,
            name,
        ) {
            return Ok(Vec::new());
        }
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
            litellm_proxy_failed_requests_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Counter,
                name: "litellm_proxy_failed_requests_metric",
                help: "Total number of failed responses from proxy - the client did not get a success response from litellm proxy",
                labels: Labels::Schema("litellm_proxy_failed_requests_metric"),
                set: BucketSet::Empty,
            })?,
            litellm_proxy_total_requests_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Counter,
                name: "litellm_proxy_total_requests_metric",
                help: "Total number of requests made to the proxy server - track number of client side requests",
                labels: Labels::Schema("litellm_proxy_total_requests_metric"),
                set: BucketSet::Empty,
            })?,
            litellm_request_total_latency_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Histogram,
                name: "litellm_request_total_latency_metric",
                help: "End-to-end latency (seconds) for a request to LiteLLM Proxy Server, from the moment the request reached the proxy through the end of processing -- includes authentication, pre-call hooks, the LLM API call, and post-call processing",
                labels: Labels::Schema("litellm_request_total_latency_metric"),
                set: BucketSet::Latency,
            })?,
            litellm_llm_api_latency_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Histogram,
                name: "litellm_llm_api_latency_metric",
                help: "Total latency (seconds) for a models LLM API call",
                labels: Labels::Schema("litellm_llm_api_latency_metric"),
                set: BucketSet::Latency,
            })?,
            litellm_llm_api_time_to_first_token_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Histogram,
                name: "litellm_llm_api_time_to_first_token_metric",
                help: "Time to first token for a models LLM API call",
                labels: Labels::Schema("litellm_llm_api_time_to_first_token_metric"),
                set: BucketSet::Latency,
            })?,
            litellm_spend_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Counter,
                name: "litellm_spend_metric",
                help: "Total spend on LLM requests",
                labels: Labels::Schema("litellm_spend_metric"),
                set: BucketSet::Empty,
            })?,
            litellm_tokens_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Counter,
                name: "litellm_total_tokens_metric",
                help: "Total number of input + output tokens from LLM requests",
                labels: Labels::Schema("litellm_total_tokens_metric"),
                set: BucketSet::Empty,
            })?,
            litellm_input_tokens_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Counter,
                name: "litellm_input_tokens_metric",
                help: "Total number of input tokens from LLM requests",
                labels: Labels::Schema("litellm_input_tokens_metric"),
                set: BucketSet::Empty,
            })?,
            litellm_output_tokens_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Counter,
                name: "litellm_output_tokens_metric",
                help: "Total number of output tokens from LLM requests",
                labels: Labels::Schema("litellm_output_tokens_metric"),
                set: BucketSet::Empty,
            })?,
            litellm_input_cached_tokens_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Counter,
                name: "litellm_input_cached_tokens_metric",
                help: "Provider-side cached input tokens (e.g. OpenAI prompt_tokens_details.cached_tokens, Anthropic cache_read_input_tokens)",
                labels: Labels::Schema("litellm_input_cached_tokens_metric"),
                set: BucketSet::Empty,
            })?,
            litellm_input_cache_creation_tokens_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Counter,
                name: "litellm_input_cache_creation_tokens_metric",
                help: "Provider-side input tokens written to prompt cache (e.g. Anthropic cache_creation_input_tokens)",
                labels: Labels::Schema("litellm_input_cache_creation_tokens_metric"),
                set: BucketSet::Empty,
            })?,
            litellm_input_audio_tokens_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Counter,
                name: "litellm_input_audio_tokens_metric",
                help: "Audio input tokens reported in prompt_tokens_details.audio_tokens",
                labels: Labels::Schema("litellm_input_audio_tokens_metric"),
                set: BucketSet::Empty,
            })?,
            litellm_output_reasoning_tokens_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Counter,
                name: "litellm_output_reasoning_tokens_metric",
                help: "Reasoning tokens reported in completion_tokens_details.reasoning_tokens",
                labels: Labels::Schema("litellm_output_reasoning_tokens_metric"),
                set: BucketSet::Empty,
            })?,
            litellm_output_audio_tokens_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Counter,
                name: "litellm_output_audio_tokens_metric",
                help: "Audio output tokens reported in completion_tokens_details.audio_tokens",
                labels: Labels::Schema("litellm_output_audio_tokens_metric"),
                set: BucketSet::Empty,
            })?,
            litellm_video_duration_seconds_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Counter,
                name: "litellm_video_duration_seconds_metric",
                help: "Seconds of video generated, from usage.duration_seconds on video generation calls",
                labels: Labels::Schema("litellm_video_duration_seconds_metric"),
                set: BucketSet::Empty,
            })?,
            litellm_images_generated_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Counter,
                name: "litellm_images_generated_metric",
                help: "Number of images generated, from the image generation response",
                labels: Labels::Schema("litellm_images_generated_metric"),
                set: BucketSet::Empty,
            })?,
            litellm_remaining_team_budget_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Gauge,
                name: "litellm_remaining_team_budget_metric",
                help: "Remaining budget for team",
                labels: Labels::Schema("litellm_remaining_team_budget_metric"),
                set: BucketSet::Empty,
            })?,
            litellm_team_max_budget_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Gauge,
                name: "litellm_team_max_budget_metric",
                help: "Maximum budget set for team",
                labels: Labels::Schema("litellm_team_max_budget_metric"),
                set: BucketSet::Empty,
            })?,
            litellm_team_budget_remaining_hours_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Gauge,
                name: "litellm_team_budget_remaining_hours_metric",
                help: "Remaining days for team budget to be reset",
                labels: Labels::Schema("litellm_team_budget_remaining_hours_metric"),
                set: BucketSet::Empty,
            })?,
            litellm_team_members_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Gauge,
                name: "litellm_team_members_metric",
                help: "Number of members in a team",
                labels: Labels::Schema("litellm_team_members_metric"),
                set: BucketSet::Empty,
            })?,
            litellm_remaining_org_budget_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Gauge,
                name: "litellm_remaining_org_budget_metric",
                help: "Remaining budget for org",
                labels: Labels::Schema("litellm_remaining_org_budget_metric"),
                set: BucketSet::Empty,
            })?,
            litellm_org_max_budget_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Gauge,
                name: "litellm_org_max_budget_metric",
                help: "Maximum budget set for org",
                labels: Labels::Schema("litellm_org_max_budget_metric"),
                set: BucketSet::Empty,
            })?,
            litellm_org_budget_remaining_hours_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Gauge,
                name: "litellm_org_budget_remaining_hours_metric",
                help: "Remaining hours for org budget to be reset",
                labels: Labels::Schema("litellm_org_budget_remaining_hours_metric"),
                set: BucketSet::Empty,
            })?,
            litellm_remaining_api_key_budget_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Gauge,
                name: "litellm_remaining_api_key_budget_metric",
                help: "Remaining budget for api key",
                labels: Labels::Schema("litellm_remaining_api_key_budget_metric"),
                set: BucketSet::Empty,
            })?,
            litellm_api_key_max_budget_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Gauge,
                name: "litellm_api_key_max_budget_metric",
                help: "Maximum budget set for api key",
                labels: Labels::Schema("litellm_api_key_max_budget_metric"),
                set: BucketSet::Empty,
            })?,
            litellm_api_key_budget_remaining_hours_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Gauge,
                name: "litellm_api_key_budget_remaining_hours_metric",
                help: "Remaining hours for api key budget to be reset",
                labels: Labels::Schema("litellm_api_key_budget_remaining_hours_metric"),
                set: BucketSet::Empty,
            })?,
            litellm_remaining_user_budget_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Gauge,
                name: "litellm_remaining_user_budget_metric",
                help: "Remaining budget for user",
                labels: Labels::Schema("litellm_remaining_user_budget_metric"),
                set: BucketSet::Empty,
            })?,
            litellm_user_max_budget_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Gauge,
                name: "litellm_user_max_budget_metric",
                help: "Maximum budget set for user",
                labels: Labels::Schema("litellm_user_max_budget_metric"),
                set: BucketSet::Empty,
            })?,
            litellm_user_budget_remaining_hours_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Gauge,
                name: "litellm_user_budget_remaining_hours_metric",
                help: "Remaining hours for user budget to be reset",
                labels: Labels::Schema("litellm_user_budget_remaining_hours_metric"),
                set: BucketSet::Empty,
            })?,
            litellm_remaining_customer_budget_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Gauge,
                name: "litellm_remaining_customer_budget_metric",
                help: "Remaining budget for customer (end user)",
                labels: Labels::Schema("litellm_remaining_customer_budget_metric"),
                set: BucketSet::Empty,
            })?,
            litellm_customer_max_budget_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Gauge,
                name: "litellm_customer_max_budget_metric",
                help: "Maximum budget set for customer (end user)",
                labels: Labels::Schema("litellm_customer_max_budget_metric"),
                set: BucketSet::Empty,
            })?,
            litellm_customer_budget_remaining_hours_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Gauge,
                name: "litellm_customer_budget_remaining_hours_metric",
                help: "Remaining hours for customer (end user) budget to be reset",
                labels: Labels::Schema("litellm_customer_budget_remaining_hours_metric"),
                set: BucketSet::Empty,
            })?,
            litellm_remaining_api_key_requests_for_model: $builder.collector(CollectorSpec {
                kind: CollectorKind::Gauge,
                name: "litellm_remaining_api_key_requests_for_model",
                help: "Remaining Requests API Key can make for model (model based rpm limit on key)",
                labels: Labels::Schema("litellm_remaining_api_key_requests_for_model"),
                set: BucketSet::Empty,
            })?,
            litellm_remaining_api_key_tokens_for_model: $builder.collector(CollectorSpec {
                kind: CollectorKind::Gauge,
                name: "litellm_remaining_api_key_tokens_for_model",
                help: "Remaining Tokens API Key can make for model (model based tpm limit on key)",
                labels: Labels::Schema("litellm_remaining_api_key_tokens_for_model"),
                set: BucketSet::Empty,
            })?,
            litellm_api_key_rate_limit_allowed_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Gauge,
                name: "litellm_api_key_rate_limit_allowed_metric",
                help: "Configured rate limit for the API Key in the current window (rpm_limit / tpm_limit), by rate_limit_type",
                labels: Labels::Schema("litellm_api_key_rate_limit_allowed_metric"),
                set: BucketSet::Empty,
            })?,
            litellm_api_key_rate_limit_used_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Gauge,
                name: "litellm_api_key_rate_limit_used_metric",
                help: "Requests or tokens the API Key has consumed in the current rate limit window, by rate_limit_type",
                labels: Labels::Schema("litellm_api_key_rate_limit_used_metric"),
                set: BucketSet::Empty,
            })?,
            litellm_team_rate_limit_allowed_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Gauge,
                name: "litellm_team_rate_limit_allowed_metric",
                help: "Configured rate limit for the Team in the current window (team rpm_limit / tpm_limit), by rate_limit_type",
                labels: Labels::Schema("litellm_team_rate_limit_allowed_metric"),
                set: BucketSet::Empty,
            })?,
            litellm_team_rate_limit_used_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Gauge,
                name: "litellm_team_rate_limit_used_metric",
                help: "Requests or tokens the Team has consumed in the current rate limit window, by rate_limit_type",
                labels: Labels::Schema("litellm_team_rate_limit_used_metric"),
                set: BucketSet::Empty,
            })?,
            litellm_remaining_requests_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Gauge,
                name: "litellm_remaining_requests_metric",
                help: "LLM Deployment Analytics - remaining requests for model, returned from LLM API Provider",
                labels: Labels::Schema("litellm_remaining_requests_metric"),
                set: BucketSet::Empty,
            })?,
            litellm_remaining_tokens_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Gauge,
                name: "litellm_remaining_tokens_metric",
                help: "remaining tokens for model, returned from LLM API Provider",
                labels: Labels::Schema("litellm_remaining_tokens_metric"),
                set: BucketSet::Empty,
            })?,
            litellm_overhead_latency_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Histogram,
                name: "litellm_overhead_latency_metric",
                help: "Latency overhead (milliseconds) added by LiteLLM processing",
                labels: Labels::Schema("litellm_overhead_latency_metric"),
                set: BucketSet::Latency,
            })?,
            litellm_overhead_with_guardrails_latency_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Histogram,
                name: "litellm_overhead_with_guardrails_latency_metric",
                help: "Total internal latency (seconds) added by LiteLLM, including pre/post-call guardrails (excludes the LLM API call)",
                labels: Labels::Schema("litellm_overhead_with_guardrails_latency_metric"),
                set: BucketSet::Latency,
            })?,
            litellm_request_queue_time_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Histogram,
                name: "litellm_request_queue_time_seconds",
                help: "Time (seconds) from request arrival at the proxy to the start of pre-call processing -- includes authentication and any ASGI-level queueing",
                labels: Labels::Schema("litellm_request_queue_time_seconds"),
                set: BucketSet::Latency,
            })?,
            litellm_guardrail_latency_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Histogram,
                name: "litellm_guardrail_latency_seconds",
                help: "Latency (seconds) for guardrail execution",
                labels: Labels::Fixed(&["guardrail_name", "status", "error_type", "hook_type"]),
                set: BucketSet::Latency,
            })?,
            litellm_guardrail_errors_total: $builder.collector(CollectorSpec {
                kind: CollectorKind::Counter,
                name: "litellm_guardrail_errors_total",
                help: "Total number of errors encountered during guardrail execution",
                labels: Labels::Fixed(&["guardrail_name", "error_type", "hook_type"]),
                set: BucketSet::Empty,
            })?,
            litellm_guardrail_requests_total: $builder.collector(CollectorSpec {
                kind: CollectorKind::Counter,
                name: "litellm_guardrail_requests_total",
                help: "Total number of guardrail invocations",
                labels: Labels::Fixed(&["guardrail_name", "status", "hook_type"]),
                set: BucketSet::Empty,
            })?,
            litellm_provider_remaining_budget_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Gauge,
                name: "litellm_provider_remaining_budget_metric",
                help: "Remaining budget for provider - used when you set provider budget limits",
                labels: Labels::Fixed(&["api_provider"]),
                set: BucketSet::Empty,
            })?,
            litellm_deployment_state: $builder.collector(CollectorSpec {
                kind: CollectorKind::Gauge,
                name: "litellm_deployment_state",
                help: "LLM Deployment Analytics - The state of the deployment: 0 = healthy, 1 = partial outage, 2 = complete outage",
                labels: Labels::Schema("litellm_deployment_state"),
                set: BucketSet::Empty,
            })?,
            litellm_deployment_tpm_limit: $builder.collector(CollectorSpec {
                kind: CollectorKind::Gauge,
                name: "litellm_deployment_tpm_limit",
                help: "Deployment TPM limit found in config",
                labels: Labels::Schema("litellm_deployment_tpm_limit"),
                set: BucketSet::Empty,
            })?,
            litellm_deployment_rpm_limit: $builder.collector(CollectorSpec {
                kind: CollectorKind::Gauge,
                name: "litellm_deployment_rpm_limit",
                help: "Deployment RPM limit found in config",
                labels: Labels::Schema("litellm_deployment_rpm_limit"),
                set: BucketSet::Empty,
            })?,
            litellm_deployment_cooled_down: $builder.collector(CollectorSpec {
                kind: CollectorKind::Counter,
                name: "litellm_deployment_cooled_down",
                help: "LLM Deployment Analytics - Number of times a deployment has been cooled down by LiteLLM load balancing logic. exception_status is the status of the exception that caused the deployment to be cooled down",
                labels: Labels::Schema("litellm_deployment_cooled_down"),
                set: BucketSet::Empty,
            })?,
            litellm_deployment_success_responses: $builder.collector(CollectorSpec {
                kind: CollectorKind::Counter,
                name: "litellm_deployment_success_responses",
                help: "LLM Deployment Analytics - Total number of successful LLM API calls via litellm",
                labels: Labels::Schema("litellm_deployment_success_responses"),
                set: BucketSet::Empty,
            })?,
            litellm_deployment_failure_responses: $builder.collector(CollectorSpec {
                kind: CollectorKind::Counter,
                name: "litellm_deployment_failure_responses",
                help: "LLM Deployment Analytics - Total number of failed LLM API calls for a specific LLM deploymeny. exception_status is the status of the exception from the llm api",
                labels: Labels::Schema("litellm_deployment_failure_responses"),
                set: BucketSet::Empty,
            })?,
            litellm_deployment_total_requests: $builder.collector(CollectorSpec {
                kind: CollectorKind::Counter,
                name: "litellm_deployment_total_requests",
                help: "LLM Deployment Analytics - Total number of LLM API calls via litellm - success + failure",
                labels: Labels::Schema("litellm_deployment_total_requests"),
                set: BucketSet::Empty,
            })?,
            litellm_deployment_latency_per_output_token: $builder.collector(CollectorSpec {
                kind: CollectorKind::Histogram,
                name: "litellm_deployment_latency_per_output_token",
                help: "LLM Deployment Analytics - Latency per output token",
                labels: Labels::Schema("litellm_deployment_latency_per_output_token"),
                set: BucketSet::ClientDefault,
            })?,
            litellm_deployment_successful_fallbacks: $builder.collector(CollectorSpec {
                kind: CollectorKind::Counter,
                name: "litellm_deployment_successful_fallbacks",
                help: "LLM Deployment Analytics - Number of successful fallback requests from primary model -> fallback model",
                labels: Labels::Schema("litellm_deployment_successful_fallbacks"),
                set: BucketSet::Empty,
            })?,
            litellm_deployment_failed_fallbacks: $builder.collector(CollectorSpec {
                kind: CollectorKind::Counter,
                name: "litellm_deployment_failed_fallbacks",
                help: "LLM Deployment Analytics - Number of failed fallback requests from primary model -> fallback model",
                labels: Labels::Schema("litellm_deployment_failed_fallbacks"),
                set: BucketSet::Empty,
            })?,
            litellm_callback_logging_failures_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Counter,
                name: "litellm_callback_logging_failures_metric",
                help: "Total number of failures when emitting logs to callbacks (e.g. s3_v2, langfuse, etc)",
                labels: Labels::Fixed(&["callback_name"]),
                set: BucketSet::Empty,
            })?,
            litellm_llm_api_failed_requests_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Counter,
                name: "litellm_llm_api_failed_requests_metric",
                help: "deprecated - use litellm_proxy_failed_requests_metric",
                labels: Labels::Schema("litellm_llm_api_failed_requests_metric"),
                set: BucketSet::Empty,
            })?,
            litellm_requests_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Counter,
                name: "litellm_requests_metric",
                help: "deprecated - use litellm_proxy_total_requests_metric. Total number of LLM calls to litellm - track total per API Key, team, user",
                labels: Labels::Schema("litellm_requests_metric"),
                set: BucketSet::Empty,
            })?,
            litellm_zero_cost_requests_total: $builder.collector(CollectorSpec {
                kind: CollectorKind::Counter,
                name: "litellm_zero_cost_requests_total",
                help: "Requests that carried usage but were logged at $0 on a model whose pricing entry has a non-zero rate, by reason (missing_pricing_key, pricing_not_applied, cost_calculation_error)",
                labels: Labels::Schema("litellm_zero_cost_requests_total"),
                set: BucketSet::Empty,
            })?,
            litellm_spend_capture_rate: $builder.collector(CollectorSpec {
                kind: CollectorKind::Gauge,
                name: "litellm_spend_capture_rate",
                help: "Share of the provider's bill LiteLLM captured as spend over the scheduled check's window (captured spend / provider bill), by api_provider; NaN when the last check produced no rate",
                labels: Labels::Schema("litellm_spend_capture_rate"),
                set: BucketSet::Empty,
            })?,
            litellm_cache_hits_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Counter,
                name: "litellm_cache_hits_metric",
                help: "Total number of LiteLLM cache hits",
                labels: Labels::Schema("litellm_cache_hits_metric"),
                set: BucketSet::Empty,
            })?,
            litellm_cache_misses_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Counter,
                name: "litellm_cache_misses_metric",
                help: "Total number of LiteLLM cache misses",
                labels: Labels::Schema("litellm_cache_misses_metric"),
                set: BucketSet::Empty,
            })?,
            litellm_cached_tokens_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Counter,
                name: "litellm_cached_tokens_metric",
                help: "Total tokens served from LiteLLM cache",
                labels: Labels::Schema("litellm_cached_tokens_metric"),
                set: BucketSet::Empty,
            })?,
            litellm_provider_cache_read_input_tokens_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Counter,
                name: "litellm_provider_cache_read_input_tokens_metric",
                help: "Total prompt/input tokens read from provider prompt cache (e.g. OpenAI/Anthropic/Gemini/Bedrock)",
                labels: Labels::Schema("litellm_provider_cache_read_input_tokens_metric"),
                set: BucketSet::Empty,
            })?,
            litellm_provider_cache_creation_input_tokens_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Counter,
                name: "litellm_provider_cache_creation_input_tokens_metric",
                help: "Total prompt/input tokens written to provider prompt cache (e.g. Anthropic/Bedrock)",
                labels: Labels::Schema("litellm_provider_cache_creation_input_tokens_metric"),
                set: BucketSet::Empty,
            })?,
            litellm_total_users_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Gauge,
                name: "litellm_total_users",
                help: "Total number of users in LiteLLM",
                labels: Labels::Fixed(&[]),
                set: BucketSet::Empty,
            })?,
            litellm_active_users_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Gauge,
                name: "litellm_active_users",
                help: "Number of billable users in LiteLLM (excludes SCIM-deactivated users)",
                labels: Labels::Fixed(&[]),
                set: BucketSet::Empty,
            })?,
            litellm_teams_count_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Gauge,
                name: "litellm_teams_count",
                help: "Total number of teams in LiteLLM",
                labels: Labels::Fixed(&[]),
                set: BucketSet::Empty,
            })?,
            litellm_managed_batch_created_total: $builder.collector(CollectorSpec {
                kind: CollectorKind::Counter,
                name: "litellm_managed_batch_created_total",
                help: "Total number of managed batches created",
                labels: Labels::Fixed(&["model", "api_provider", "user", "user_email", "api_key_alias"]),
                set: BucketSet::Empty,
            })?,
            litellm_managed_file_size_bytes: $builder.collector(CollectorSpec {
                kind: CollectorKind::Gauge,
                name: "litellm_managed_file_size_bytes",
                help: "Size of the most recent managed batch file in bytes (last-seen value per label combination)",
                labels: Labels::Fixed(&["purpose", "file_type", "model", "api_provider", "user"]),
                set: BucketSet::Empty,
            })?,
            litellm_managed_batch_duration_seconds: $builder.collector(CollectorSpec {
                kind: CollectorKind::Histogram,
                name: "litellm_managed_batch_duration_seconds",
                help: "Duration of completed managed batches in seconds (completed_at - created_at)",
                labels: Labels::Fixed(&["model", "api_provider"]),
                set: BucketSet::BatchDuration,
            })?,
            litellm_managed_file_created_total: $builder.collector(CollectorSpec {
                kind: CollectorKind::Counter,
                name: "litellm_managed_file_created_total",
                help: "Total number of managed files created",
                labels: Labels::Fixed(&["model", "api_provider", "user", "user_email", "api_key_alias"]),
                set: BucketSet::Empty,
            })?,
            litellm_managed_file_deleted_total: $builder.collector(CollectorSpec {
                kind: CollectorKind::Counter,
                name: "litellm_managed_file_deleted_total",
                help: "Total number of managed file deletions (success or blocked)",
                labels: Labels::Fixed(&["result"]),
                set: BucketSet::Empty,
            })?,
            litellm_check_batch_cost_jobs_polled: $builder.collector(CollectorSpec {
                kind: CollectorKind::Gauge,
                name: "litellm_check_batch_cost_jobs_polled",
                help: "Number of unprocessed batches found by the last CheckBatchCost poll",
                labels: Labels::Fixed(&[]),
                set: BucketSet::Empty,
            })?,
            litellm_check_batch_cost_jobs_processed_total: $builder.collector(CollectorSpec {
                kind: CollectorKind::Counter,
                name: "litellm_check_batch_cost_jobs_processed_total",
                help: "Total number of batches successfully cost-tracked by CheckBatchCost",
                labels: Labels::Fixed(&["model", "api_provider"]),
                set: BucketSet::Empty,
            })?,
            litellm_check_batch_cost_errors_total: $builder.collector(CollectorSpec {
                kind: CollectorKind::Counter,
                name: "litellm_check_batch_cost_errors_total",
                help: "Total number of errors in CheckBatchCost by error type",
                labels: Labels::Fixed(&["error_type"]),
                set: BucketSet::Empty,
            })?,
            litellm_check_batch_cost_last_run_timestamp: $builder.collector(CollectorSpec {
                kind: CollectorKind::Gauge,
                name: "litellm_check_batch_cost_last_run_timestamp",
                help: "Unix timestamp of the last CheckBatchCost job run",
                labels: Labels::Fixed(&[]),
                set: BucketSet::Empty,
            })?,
            litellm_mcp_tool_calls_total: $builder.collector(CollectorSpec {
                kind: CollectorKind::Counter,
                name: "litellm_mcp_tool_calls_total",
                help: "Total MCP tool calls, segmented by tool and server name",
                labels: Labels::Schema("litellm_mcp_tool_calls_total"),
                set: BucketSet::Empty,
            })?,
            litellm_mcp_tool_call_spend_metric: $builder.collector(CollectorSpec {
                kind: CollectorKind::Counter,
                name: "litellm_mcp_tool_call_spend_metric",
                help: "Total spend on MCP tool calls, segmented by tool and server name",
                labels: Labels::Schema("litellm_mcp_tool_call_spend_metric"),
                set: BucketSet::Empty,
            })?,
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
        let _caller_identity =
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
            Self::NoOp(_) | Self::Excluded(_) => Ok(ChildRemoval::Unsupported),
            Self::Real(metric) => Ok(match metric.remove_series(label_values) {
                SeriesRemoval::Removed => ChildRemoval::Removed,
                SeriesRemoval::Absent => ChildRemoval::Absent,
                SeriesRemoval::Unsupported => ChildRemoval::Unsupported,
            }),
        }
    }
}
