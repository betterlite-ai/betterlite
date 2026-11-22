use betterlite_models::call_helpers::metadata::{MetadataBucket, metadata_variable_name};
use betterlite_models::decisions::Json;
use betterlite_prometheus::label_value_record::{LabelDumpValue, UserApiKeyLabelValues};

use crate::fields::payload::records::StandardLoggingMetadata;
use crate::logging_metadata::metadata::get_standard_logging_metadata;
use crate::prometheus_label_projection::label_factory_context::PrometheusLabelFactoryContext;
use crate::prometheus_label_projection::prometheus_label_factory::LabelSource;

use super::metric::DeploymentMetric;
use super::metrics::{DeploymentMetrics, DeploymentResult};
use super::requests::FallbackRequest;
use super::values::{fallback_exception_status, push_identity, push_text};

pub fn log_success_fallback_event<G: DeploymentMetrics>(
    metrics: &G,
    request: &FallbackRequest<'_>,
) -> DeploymentResult<G> {
    tracing::debug!(
        "Prometheus: log_success_fallback_event, original_model_group: {}",
        request.original_model_group
    );
    record_fallback(metrics, request, DeploymentMetric::SuccessfulFallbacks)
}

pub fn log_failure_fallback_event<G: DeploymentMetrics>(
    metrics: &G,
    request: &FallbackRequest<'_>,
) -> DeploymentResult<G> {
    tracing::debug!(
        "Prometheus: log_failure_fallback_event, original_model_group: {}",
        request.original_model_group
    );
    record_fallback(metrics, request, DeploymentMetric::FailedFallbacks)
}

fn record_fallback<G: DeploymentMetrics>(
    metrics: &G,
    request: &FallbackRequest<'_>,
    metric: DeploymentMetric,
) -> DeploymentResult<G> {
    // get_metadata_variable_name_from_kwargs: litellm_metadata when the kwargs have that key, else metadata
    let metadata = match metadata_variable_name(request.metadata) {
        MetadataBucket::Metadata => request.metadata.metadata.as_ref(),
        MetadataBucket::LitellmMetadata => request.metadata.litellm_metadata.as_ref(),
    };
    let standard = get_standard_logging_metadata(metadata)?;
    let values = fallback_values(metrics, request, &standard);
    let context = PrometheusLabelFactoryContext::new(
        &values,
        metrics.settings(),
        metrics.end_user_settings(),
    )?;
    metrics.increment_counter(metric, &values, LabelSource::Context(&context), request.now)
}

fn fallback_values<G: DeploymentMetrics>(
    metrics: &G,
    request: &FallbackRequest<'_>,
    standard: &StandardLoggingMetadata,
) -> UserApiKeyLabelValues {
    let mut entries: Vec<(String, LabelDumpValue)> = Vec::new();
    let requested = Json::Text(request.original_model_group.to_owned());
    let requested = metrics.bounded_model_label(Some(&requested), true, request.catalog);
    push_text(&mut entries, "requested_model", requested.as_deref());
    push_text(&mut entries, "fallback_model", request.model);
    push_identity(
        &mut entries,
        standard.user_api_key_hash.as_deref(),
        standard.user_api_key_alias.as_deref(),
        standard.user_api_key_team_id.as_deref(),
        standard.user_api_key_team_alias.as_deref(),
    );
    let status = fallback_exception_status(request.exception);
    push_text(&mut entries, "exception_status", Some(&status));
    let class = metrics.exception_class(request.exception);
    push_text(&mut entries, "exception_class", Some(&class));
    entries.push((
        "tags".to_owned(),
        LabelDumpValue::Strings(request.tags.to_vec()),
    ));
    UserApiKeyLabelValues::from_entries(&entries)
}
