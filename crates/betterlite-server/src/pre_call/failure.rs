use serde_json::{Map, Value};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyFailure {
    pub message: String,
    pub error_type: String,
    pub param: Option<String>,
    pub code: u16,
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum PreCallFailure {
    #[error("proxy exception {}: {}", .0.error_type, .0.message)]
    Proxy(ProxyFailure),
    #[error("rate limited: {}", .0.message)]
    RateLimited(ProxyFailure),
    #[error("http exception {status}: {detail}")]
    Http { status: u16, detail: Value },
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
#[error("the pre-call step failed")]
pub struct PreCallError {
    #[source]
    pub failure: PreCallFailure,
    pub data: Map<String, Value>,
}
