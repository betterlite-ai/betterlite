use betterlite_models::call_helpers::metadata::{
    MetadataBucket, MetadataMap, MetadataSlots, metadata_variable_name,
};
use betterlite_prometheus::label_value_record::UserApiKeyLabelValues;

use crate::fields::payload::records::StandardLoggingMetadata;
use crate::logging_metadata::metadata::get_standard_logging_metadata;
use crate::prometheus_helpers::bounded_series_tracker::RemovableMetric;
use crate::prometheus_label_projection::label_factory_context::PrometheusLabelFactoryContext;
use crate::prometheus_label_projection::prometheus_label_factory::LabelSource;

use super::error::PrometheusDeploymentError;
use super::metric::DeploymentMetric;
use super::metrics::DeploymentMetrics;
use super::requests::FallbackRequest;
use super::values::fallback_exception_status;

pub fn log_success_fallback_event<Gauges>(
    metrics: &Gauges,
    request: &FallbackRequest<'_>,
) -> Result<(), PrometheusDeploymentError<<Gauges::Gauge as RemovableMetric>::Error>>
where
    Gauges: DeploymentMetrics,
{
    tracing::debug!(
        "Prometheus: log_success_fallback_event, original_model_group: {}",
        request.original_model_group
    );
    record_fallback(metrics, request, DeploymentMetric::SuccessfulFallbacks)
}

pub fn log_failure_fallback_event<Gauges>(
    metrics: &Gauges,
    request: &FallbackRequest<'_>,
) -> Result<(), PrometheusDeploymentError<<Gauges::Gauge as RemovableMetric>::Error>>
where
    Gauges: DeploymentMetrics,
{
    tracing::debug!(
        "Prometheus: log_failure_fallback_event, original_model_group: {}",
        request.original_model_group
    );
    record_fallback(metrics, request, DeploymentMetric::FailedFallbacks)
}

fn record_fallback<Gauges>(
    metrics: &Gauges,
    request: &FallbackRequest<'_>,
    metric: DeploymentMetric,
) -> Result<(), PrometheusDeploymentError<<Gauges::Gauge as RemovableMetric>::Error>>
where
    Gauges: DeploymentMetrics,
{
    let metadata = bucket_metadata(request.metadata);
    let standard = get_standard_logging_metadata(metadata)?;
    let values = fallback_values(metrics, request, &standard);
    let context = PrometheusLabelFactoryContext::new(
        &values,
        metrics.settings(),
        metrics.end_user_settings(),
    )?;
    metrics.increment_counter(metric, &values, LabelSource::Context(&context), request.now)
}

// get_metadata_variable_name_from_kwargs: litellm_metadata when the kwargs have that key, else metadata
fn bucket_metadata(slots: &MetadataSlots) -> Option<&MetadataMap> {
    match metadata_variable_name(slots) {
        MetadataBucket::Metadata => slots.metadata.as_ref(),
        MetadataBucket::LitellmMetadata => slots.litellm_metadata.as_ref(),
    }
}

fn fallback_values<Gauges>(
    metrics: &Gauges,
    request: &FallbackRequest<'_>,
    standard: &StandardLoggingMetadata,
) -> UserApiKeyLabelValues
where
    Gauges: DeploymentMetrics,
{
    UserApiKeyLabelValues {
        requested_model: metrics
            .bounded_model_name(Some(request.original_model_group), request.catalog),
        fallback_model: request.model.map(str::to_owned),
        hashed_api_key: standard.user_api_key_hash.clone(),
        api_key_alias: standard.user_api_key_alias.clone(),
        team: standard.user_api_key_team_id.clone(),
        team_alias: standard.user_api_key_team_alias.clone(),
        exception_status: Some(fallback_exception_status(request.exception)),
        exception_class: Some(metrics.exception_class(request.exception)),
        tags: request.tags.to_vec(),
        ..UserApiKeyLabelValues::default()
    }
}
