use thiserror::Error;

#[derive(Debug, Error)]
pub enum PredictionError {
    // LiteLLM raises InternalServerError, status 500, when the key is missing
    #[error("image generation response does not contain 'predictions'")]
    MissingPredictions,
    // `predictions[0]` on an empty list
    #[error("image generation response has no prediction to inspect")]
    EmptyPredictions,
    // prediction["bytesBase64Encoded"] on a prediction without the key
    #[error("prediction {index} has no 'bytesBase64Encoded'")]
    MissingImageKey { index: usize },
    #[error("'predictions' must be a list of objects, got {found} at {path}")]
    Shape { path: String, found: &'static str },
    #[error("'bytesBase64Encoded' of prediction {index} must be a string or null")]
    ImageType { index: usize },
}
