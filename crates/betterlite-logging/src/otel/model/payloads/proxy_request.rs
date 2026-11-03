use super::RequestIdentity;

#[derive(Clone, Debug)]
pub struct ProxyRequestSpanData {
    pub http_method: String,
    pub route: String,
    pub url_path: Option<String>,
    pub status_code: Option<i64>,
    pub identity: Option<RequestIdentity>,
}

#[path = "proxy_request/tests.rs"]
mod tests;
