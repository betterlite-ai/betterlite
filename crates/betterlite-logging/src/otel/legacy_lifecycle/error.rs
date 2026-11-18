use crate::otel::legacy_exporters::error::LegacyExporterError;
use crate::otel::legacy_metrics::error::LegacyMetricsError;
use crate::otel::model::metadata::wire::MetadataError;
use crate::private_json::error::PrivateJsonError;

#[derive(Debug, thiserror::Error)]
pub enum LegacyLifecycleError {
    #[error(transparent)]
    Exporter(#[from] LegacyExporterError),
    #[error(transparent)]
    Metrics(#[from] LegacyMetricsError),
    #[error(transparent)]
    Metadata(#[from] MetadataError),
    #[error("standard_callback_dynamic_params must be an object")]
    DynamicParams,
    #[error("the guardrail entry field {field} must be {expected}")]
    GuardrailField {
        field: &'static str,
        expected: &'static str,
    },
    #[error("cannot serialize the guardrail attribute {attribute}: {source}")]
    GuardrailJson {
        attribute: &'static str,
        #[source]
        source: PrivateJsonError,
    },
}
