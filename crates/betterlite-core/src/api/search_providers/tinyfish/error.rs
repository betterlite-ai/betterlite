use crate::api::search_providers::google_pse::wire::WireError;
use betterlite_openai::error::ProviderError;
use betterlite_openai::search::credentials::CredentialError;
use thiserror::Error;
#[derive(Debug, Error)]
pub enum TinyfishSearchError {
    #[error("TINYFISH_API_KEY is not set. Set `TINYFISH_API_KEY` environment variable.")]
    MissingApiKey,
    #[error(transparent)]
    Credential(#[from] CredentialError),
    #[error("{}", .0.message)]
    Provider(Box<ProviderError>),
    #[error("the TinyFish search reply is larger than {limit} bytes")]
    ReplyTooLarge { limit: usize },
    #[error(
        "the TinyFish search parameter `{field}` is null, and a query parameter must be a string, number or bool"
    )]
    ParamType { field: String },
    #[error(transparent)]
    Wire(#[from] WireError),
}
