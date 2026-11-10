use std::fmt;
use std::io;

use crate::azure_client_secret::transport::{HttpMethod, HttpReply};

#[derive(Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: HttpMethod,
    pub url: String,
    pub query: Vec<(String, String)>,
    pub headers: Vec<(String, String)>,
    pub form: Vec<(String, String)>,
}

// the password travels in the form body, and a Debug of the request must not put it in a log line
impl fmt::Debug for HttpRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let form: Vec<(&str, &str)> = self
            .form
            .iter()
            .map(|(name, value)| match name.as_str() {
                "password" => (name.as_str(), "[redacted]"),
                _ => (name.as_str(), value.as_str()),
            })
            .collect();
        f.debug_struct("HttpRequest")
            .field("method", &self.method)
            .field("url", &self.url)
            .field("query", &self.query)
            .field("headers", &self.headers)
            .field("form", &form)
            .finish()
    }
}

pub trait Transport: Send + Sync {
    fn send(&self, request: &HttpRequest) -> io::Result<HttpReply>;
}
