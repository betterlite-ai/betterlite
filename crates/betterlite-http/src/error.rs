use thiserror::Error;

#[derive(Debug, Error)]
pub enum HttpError {
    #[error("{field_name} is required")]
    Required { field_name: String },
    #[error("{field_name} cannot be a dot path segment")]
    DotSegment { field_name: String },
    #[error("the proxy for this request uses {scheme}://, and only http:// proxies are supported")]
    UnsupportedProxyScheme { scheme: String },
    #[error("the {name} proxy URL is not valid")]
    InvalidProxyUrl { name: String },
    #[error("cannot build the HTTP client")]
    Client(#[source] reqwest::Error),
    #[error("the request failed")]
    Request(#[source] reqwest::Error),
}
