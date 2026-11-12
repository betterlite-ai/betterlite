use thiserror::Error;

#[derive(Debug, Error)]
pub enum MultimodalEmbeddingError {
    #[error(
        "the multimodal embedding input element at index {index} is neither a string nor an object"
    )]
    UnsupportedElement { index: usize },
    #[error("the multimodal embedding input must be a string or a list")]
    UnsupportedInput,
    #[error("the multimodal embedding request instances are not a list")]
    InstancesNotList,
    #[error("the multimodal embedding request instance is not an object")]
    InstanceNotObject,
    #[error("the multimodal embedding request {field} field is not text")]
    NotText { field: &'static str },
    #[error("the multimodal embedding request {field} field is not an object")]
    NotObject { field: &'static str },
    #[error("the multimodal embedding response is {size} bytes, over the {limit} byte limit")]
    ResponseTooLarge { size: usize, limit: usize },
    #[error("the multimodal embedding response is not valid JSON: {source}")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the multimodal embedding response arrived with status {status}")]
    Status { status: u16 },
    #[error("the multimodal embedding response has no predictions")]
    MissingPredictions,
    #[error("the multimodal embedding predictions are not a list")]
    PredictionsNotList,
    #[error("the prediction at index {index} is not an object")]
    PredictionNotObject { index: usize },
    #[error("the prediction at index {index} has no {field} list")]
    NotVector { field: &'static str, index: usize },
    #[error("the prediction at index {index} has a {field} that is not a number")]
    NotNumber { field: &'static str, index: usize },
    #[error("the prediction at index {index} has a video segment without an embedding")]
    MissingSegmentEmbedding { index: usize },
    #[error("the embedding index overflows")]
    IndexOverflow,
    #[error("the character count overflows")]
    CountOverflow,
    #[error("a video duration overflows the decimal range")]
    DurationOverflow,
}

impl MultimodalEmbeddingError {
    // LiteLLM raises InternalServerError for a reply without predictions, and every other reply failure reaches the proxy as a 500
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::UnsupportedElement { .. }
            | Self::UnsupportedInput
            | Self::InstancesNotList
            | Self::InstanceNotObject
            | Self::NotText { .. }
            | Self::NotObject { .. } => 400,
            Self::ResponseTooLarge { .. }
            | Self::Decode { .. }
            | Self::Status { .. }
            | Self::MissingPredictions
            | Self::PredictionsNotList
            | Self::PredictionNotObject { .. }
            | Self::NotVector { .. }
            | Self::NotNumber { .. }
            | Self::MissingSegmentEmbedding { .. }
            | Self::IndexOverflow
            | Self::CountOverflow
            | Self::DurationOverflow => 500,
        }
    }

    #[must_use]
    pub const fn error_type(&self) -> &'static str {
        match self {
            Self::UnsupportedElement { .. }
            | Self::UnsupportedInput
            | Self::InstancesNotList
            | Self::InstanceNotObject
            | Self::NotText { .. }
            | Self::NotObject { .. } => "invalid_request_error",
            Self::ResponseTooLarge { .. }
            | Self::Decode { .. }
            | Self::Status { .. }
            | Self::MissingPredictions
            | Self::PredictionsNotList
            | Self::PredictionNotObject { .. }
            | Self::NotVector { .. }
            | Self::NotNumber { .. }
            | Self::MissingSegmentEmbedding { .. }
            | Self::IndexOverflow
            | Self::CountOverflow
            | Self::DurationOverflow => "internal_server_error",
        }
    }
}
