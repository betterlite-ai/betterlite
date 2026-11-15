use betterlite_openai::api::image_contracts::records::ImageHeaders;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum FalImageError {
    #[error("FAL_AI_API_KEY is not set")]
    MissingApiKey,
    #[error("parameters {parameters:?} are not supported for model {model}")]
    UnsupportedParameters {
        parameters: Vec<String>,
        model: String,
    },
    #[error("the Fal reply at HTTP {status} is larger than {limit} bytes")]
    ReplyTooLarge {
        status: u16,
        headers: ImageHeaders,
        limit: usize,
    },
    #[error("the Fal reply at HTTP {status} is not JSON")]
    ReplyDecode {
        status: u16,
        headers: ImageHeaders,
        #[source]
        source: serde_json::Error,
    },
    #[error("the Fal reply at HTTP {status} is not a JSON object")]
    ReplyNotAnObject { status: u16, headers: ImageHeaders },
}
