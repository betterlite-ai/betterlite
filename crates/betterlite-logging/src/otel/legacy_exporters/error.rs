use crate::otel::{legacy_exporters::headers::HeadersError, plumbing::exporters::ExporterError};

#[derive(Debug, thiserror::Error)]
pub enum LegacyExporterError {
    #[error(transparent)]
    Headers(#[from] HeadersError),
    #[error(transparent)]
    Exporter(#[from] ExporterError),
}
