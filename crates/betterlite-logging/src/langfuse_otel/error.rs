use crate::otel::legacy_exporters::error::LegacyExporterError;

#[derive(Debug, thiserror::Error)]
pub enum LangfuseOtelError {
    #[error(
        "LANGFUSE_PUBLIC_KEY and LANGFUSE_SECRET_KEY must be set for Langfuse OpenTelemetry integration."
    )]
    MissingCredentials,
    #[error("standard_callback_dynamic_params must be an object")]
    DynamicParamsType,
    #[error("standard_callback_dynamic_params.{field} must be a string")]
    DynamicFieldType { field: &'static str },
    #[error("the OpenTelemetry logger could not be initialized")]
    Init {
        #[source]
        source: LegacyExporterError,
    },
}
