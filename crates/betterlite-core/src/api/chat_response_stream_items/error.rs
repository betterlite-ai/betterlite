use betterlite_streaming::streamed_response_assembly::AssemblyError;
use thiserror::Error;

use crate::api::chat_response_stream_tools::error::StreamToolsError;

#[derive(Debug, Error)]
pub enum StreamItemsError {
    #[error(transparent)]
    Stream(#[from] StreamToolsError),
    #[error("the response fields are invalid: {source}")]
    Response {
        #[from]
        source: serde_path_to_error::Error<serde_json::Error>,
    },
    #[error("the response data could not be encoded: {source}")]
    Encode {
        #[from]
        source: serde_json::Error,
    },
    #[error("the {field} metadata is not a string")]
    Metadata { field: &'static str },
    #[error("the responses API request is not a JSON object")]
    ResponseNotObject,
    #[error("the completion has no first choice")]
    MissingChoice,
    #[error("the completion content is not a string")]
    ContentNotText,
    #[error("the completion annotations are not a list")]
    AnnotationsNotList,
    #[error("the completion annotations are invalid: {source}")]
    Annotation { source: serde_json::Error },
    #[error("the hidden parameters are not a JSON object")]
    HiddenParamsNotObject,
    #[error("the hidden provider_specific_fields are not a JSON object")]
    ProviderFieldsNotObject,
    #[error("no assembled model response is stored")]
    MissingModelResponse,
    #[error("the default top_p is not a finite number")]
    DefaultTopPNotFinite,
    #[error("the system clock reads a time before the Unix epoch")]
    ClockBeforeEpoch,
    #[error("the system clock reads past the i64 second range")]
    ClockOutOfRange,
    #[error(transparent)]
    Assembly(#[from] AssemblyError),
}
