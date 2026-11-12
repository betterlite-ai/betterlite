use betterlite_http::error::HttpError;
use indexmap::IndexMap;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StudioInteractionError {
    #[error(
        "Google API key is required. Set GOOGLE_API_KEY or GEMINI_API_KEY environment variable."
    )]
    MissingApiKey,
    #[error("Either 'model' or 'agent' must be provided")]
    MissingTarget,
    #[error("the generation_config image_config must be an object")]
    ImageConfigNotObject,
    #[error("the interaction_id cannot be used as a URL segment")]
    InteractionId {
        #[source]
        source: HttpError,
    },
    #[error("the interaction reply is {size} bytes, over the {limit} byte limit")]
    ResponseTooLarge { size: usize, limit: usize },
    #[error("Gemini answered with status {status}: {body}")]
    Gemini {
        status: u16,
        body: String,
        headers: IndexMap<String, String>,
    },
    #[error("the interaction record is not valid")]
    Record {
        #[source]
        source: serde_json::Error,
    },
}
