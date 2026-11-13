use betterlite_http::error::HttpError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::error::ProviderError;
use betterlite_prompts::media_metadata::error::MediaMetadataError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum FilesError {
    #[error("File data is required")]
    FileDataRequired,
    #[error(
        "Anthropic API key is required. Set ANTHROPIC_API_KEY or ANTHROPIC_AUTH_TOKEN environment \
         variable or pass api_key parameter."
    )]
    MissingApiKey,
    #[error("the {name} header value is not a valid HTTP header value")]
    InvalidHeader { name: String },
    #[error("the api_base is not a URL")]
    InvalidBase {
        #[source]
        source: url::ParseError,
    },
    #[error(transparent)]
    Segment(#[from] HttpError),
    #[error(transparent)]
    Media(#[from] MediaMetadataError),
    #[error("the file reply is not an Anthropic file object")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the request could not be sent")]
    Transport {
        #[source]
        source: TransportError,
    },
    #[error("{}", .0.message)]
    Provider(Box<ProviderError>),
}
