use betterlite_env::EnvError;
use betterlite_http::error::HttpError;
// What stops a lookup before the proxy is asked; the status line then shows only the fallback text
#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("the gateway URL is not valid: {0}")]
    InvalidUrl(#[source] url::ParseError),
    #[error("cannot resolve the proxy settings: {0}")]
    Proxy(#[source] EnvError),
    #[error("cannot prepare the request: {0}")]
    Http(#[source] HttpError),
}
