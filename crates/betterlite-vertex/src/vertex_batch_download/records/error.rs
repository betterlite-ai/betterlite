use thiserror::Error;

use crate::gemini_response::error::GenerateContentError;
use crate::vertex_batch_rows::error::BatchRowsError;

#[derive(Debug, Error)]
pub(crate) enum RecordsError {
    #[error("the vertex batch output row is not usable: {0}")]
    Row(#[from] BatchRowsError),
    #[error("the response of the vertex batch output row is not a JSON object")]
    Response,
    #[error("the request labels of the vertex batch output row are not an object")]
    Labels,
    #[error("the modelVersion of the vertex batch response is not a string")]
    ModelVersion,
    #[error("the converted batch output row cannot be encoded: {0}")]
    Encode(#[source] serde_json::Error),
}

#[derive(Debug, Error)]
pub(crate) enum ConversionError {
    #[error(transparent)]
    Gemini(#[from] GenerateContentError),
    #[error("the converted batch response is not a JSON object")]
    Shape,
    #[error("the converted batch response holds streamed choices")]
    StreamedChoices,
    #[error("the converted batch response cannot be encoded: {0}")]
    Encode(#[source] serde_json::Error),
}
