use std::error::Error;

use crate::model_metadata_resolution::error::ModelMetadataError;

#[derive(Debug, thiserror::Error)]
pub enum ModelRecordError {
    #[error("model metadata is not valid JSON: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("model metadata is not an object")]
    NotAnObject,
    #[error("model metadata already carries supported_openai_params")]
    DuplicateSupportedParameters,
}

#[derive(Debug, thiserror::Error)]
pub enum FullMetadataError<ParameterError>
where
    ParameterError: Error + 'static,
{
    #[error("the supported parameter query failed: {source}")]
    Parameters {
        #[source]
        source: ParameterError,
    },
    #[error(transparent)]
    Metadata(#[from] ModelMetadataError),
    #[error(transparent)]
    Record(#[from] ModelRecordError),
}
