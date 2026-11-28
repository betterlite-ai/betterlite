use std::string::FromUtf8Error;

use betterlite_http::proxy::transport_error::TransportError;
use thiserror::Error;

use crate::public_endpoints::blog_feed_parser::ParseFailure;

#[derive(Debug, Error)]
pub enum BlogFeedError {
    #[error(transparent)]
    Fetch(#[from] FetchError),
    #[error(transparent)]
    Parse(#[from] ParseFailure),
    #[error(transparent)]
    Local(#[from] LocalPostsError),
}

#[derive(Debug, Error)]
pub enum FetchError {
    #[error("the blog posts URL {url} is not a URL")]
    Url {
        url: String,
        #[source]
        source: url::ParseError,
    },
    #[error("the RSS request failed")]
    Request(#[source] TransportError),
    #[error("the RSS response returned HTTP status {status}")]
    Status { status: u16 },
    #[error("the RSS response is larger than {limit} bytes")]
    TooLarge { limit: usize },
    #[error("the RSS response is not valid UTF-8")]
    Utf8(#[source] FromUtf8Error),
}

#[derive(Debug, Error)]
pub enum LocalPostsError {
    #[error("the bundled blog posts are not valid JSON")]
    Decode(#[from] serde_json::Error),
    #[error("the bundled blog posts are missing")]
    Missing,
}
