use std::future::Future;
use std::time::Duration;

use indexmap::IndexMap;

use crate::primitives::json_value::JsonValue;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Delete,
    Patch,
}

impl HttpMethod {
    // Takes the upper-cased method name.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "GET" => Some(Self::Get),
            "POST" => Some(Self::Post),
            "PUT" => Some(Self::Put),
            "DELETE" => Some(Self::Delete),
            "PATCH" => Some(Self::Patch),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum RequestBody {
    Empty,
    Json(IndexMap<String, JsonValue>),
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpReply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedDestination {
    pub url: String,
    pub host_header: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutboundRequest {
    pub method: HttpMethod,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: RequestBody,
    pub timeout: Duration,
    pub connect_timeout: Duration,
    pub follow_redirects: bool,
}

// The message is what LiteLLM puts in the result's error field, so custom code sees the same text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportFailure {
    Refused { message: String },
    TimedOut,
    Failed { message: String },
}

// Backed by the user-URL validation and the pooled client: validate_url, async_safe_get and a client built with follow_redirects off while validation is on.
pub trait OutboundHttp: Sync {
    fn user_url_validation(&self) -> bool;

    fn validate_url(
        &self,
        url: &str,
    ) -> impl Future<Output = Result<ValidatedDestination, TransportFailure>> + Send;

    fn safe_get(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<HttpReply, TransportFailure>> + Send;

    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<HttpReply, TransportFailure>> + Send;
}
