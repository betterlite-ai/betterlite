use betterlite_http::user_url::error::UrlParseError;
use thiserror::Error;

use crate::azure_urls::AzureUrlError;
use crate::credentials::selection::AzureTokenError;

#[derive(Debug, Error)]
pub enum ResourceUrlError {
    #[error("api_base cannot be parsed")]
    Parse(#[source] UrlParseError),
    #[error("the Azure resource URL cannot be built")]
    Url(#[source] AzureUrlError),
    #[error("the Azure resource credentials cannot be resolved")]
    Token(#[source] AzureTokenError),
}
