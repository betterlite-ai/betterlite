use std::time::Duration;

use http::Method;
use http::header::HeaderMap;
use url::Url;

// The request parts an outgoing pass-through call needs before B173's transport builds and sends it
#[derive(Debug, Clone)]
pub struct UpstreamTarget {
    pub method: Method,
    pub url: Url,
    pub headers: HeaderMap,
    pub query: Vec<(String, String)>,
    pub deadline: Duration,
}
