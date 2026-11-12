use std::num::TryFromIntError;

use thiserror::Error;

pub const INTERNAL_SERVER_ERROR: &str = "internal_server_error";

#[derive(Debug, Error)]
pub enum MultimodalEmbeddingError {
    #[error(
        "the multimodal embedding input element at index {index} is neither a string nor an object"
    )]
    UnsupportedElement { index: usize },
    #[error("the multimodal embedding input must be a string or a list")]
    UnsupportedInput,
    #[error("the multimodal embedding request {0} must be {1}")]
    Request(&'static str, &'static str),
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
    #[error("the {0} of prediction {2} must be {1}")]
    Field(&'static str, &'static str, usize),
    #[error("the multimodal embedding {0} overflows")]
    Overflow(&'static str),
    #[error("the multimodal embedding {0} does not fit its integer range: {1}")]
    IntegerRange(&'static str, #[source] TryFromIntError),
    #[error("the multimodal embedding {0} of prediction {1} is not a decimal number: {2}")]
    Decimal(&'static str, usize, #[source] rust_decimal::Error),
}

impl MultimodalEmbeddingError {
    // LiteLLM raises InternalServerError for a reply without predictions, and every other reply failure reaches the proxy as a 500
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::UnsupportedElement { .. } | Self::UnsupportedInput | Self::Request(..) => 400,
            Self::ResponseTooLarge { .. }
            | Self::Decode { .. }
            | Self::Status { .. }
            | Self::MissingPredictions
            | Self::PredictionsNotList
            | Self::PredictionNotObject { .. }
            | Self::Field(..)
            | Self::Overflow(_)
            | Self::IntegerRange(..)
            | Self::Decimal(..) => 500,
        }
    }
}
