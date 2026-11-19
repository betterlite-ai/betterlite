use crate::otel::legacy_lifecycle::error::LegacyLifecycleError;
use crate::otel::model::metadata::wire::MetadataError;

#[derive(Debug, thiserror::Error)]
pub enum ServiceCallbackError {
    #[error(transparent)]
    Lifecycle(#[from] LegacyLifecycleError),
    #[error(transparent)]
    Metadata(#[from] MetadataError),
}
