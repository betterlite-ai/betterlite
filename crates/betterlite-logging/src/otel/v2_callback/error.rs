use crate::otel::legacy_metrics::error::LegacyMetricsError;
use crate::otel::mappers::registry::MapperError;
use crate::otel::plumbing::exporters::ExporterError;

#[derive(Debug, thiserror::Error)]
pub enum CallbackError {
    #[error(transparent)]
    Exporter(#[from] ExporterError),
    #[error(transparent)]
    Mapper(#[from] MapperError),
    #[error(transparent)]
    Metrics(#[from] LegacyMetricsError),
}
