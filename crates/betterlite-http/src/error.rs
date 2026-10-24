use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum UrlPathError {
    #[error("{field_name} is required")]
    Required { field_name: String },
    #[error("{field_name} cannot be a dot path segment")]
    DotSegment { field_name: String },
}

#[derive(Debug, Error)]
pub enum ProxyError {
    #[error("the proxy for this request uses {scheme}://, and only http:// proxies are supported")]
    UnsupportedScheme { scheme: String },
    #[error("the {name} proxy URL is not valid")]
    InvalidUrl { name: String },
    #[error("cannot build the HTTP client")]
    Client {
        #[source]
        source: reqwest::Error,
    },
    #[error("the request failed")]
    Request {
        #[source]
        source: reqwest::Error,
    },
}
