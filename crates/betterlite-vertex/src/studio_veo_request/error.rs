use thiserror::Error;

use crate::vertex_veo_request::error::VeoRequestError;

#[derive(Debug, Error)]
pub enum StudioVeoError {
    #[error(
        "GEMINI_API_KEY or GOOGLE_API_KEY is required for Veo video generation. Set it via environment variable or pass it as api_key parameter."
    )]
    MissingApiKey,
    #[error("the image must be a dict or a binary file-like object")]
    ImageNotAFile,
    #[error("the image file could not be read")]
    ImageRead(#[source] VeoRequestError),
    #[error("the video parameter {path} does not match the Gemini model: {source}")]
    Parameters {
        path: String,
        #[source]
        source: serde_json::Error,
    },
}
