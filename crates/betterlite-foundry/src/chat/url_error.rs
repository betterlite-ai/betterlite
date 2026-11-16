use betterlite_openai::openai_base_config::forwarding::BaseConfigError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum UrlError {
    #[error(
        "api_base is required for Azure AI Studio. Please set the api_base parameter. Passed `api_base=None`"
    )]
    MissingApiBase,
    #[error("api_base is not a valid URL")]
    InvalidUrl(#[source] url::ParseError),
    #[error("api_base cannot take the route")]
    Compose(#[source] BaseConfigError),
}
