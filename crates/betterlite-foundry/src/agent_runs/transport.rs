use std::fmt;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_prompts::arguments::json::Json;

use crate::agent_requests::prepare::Headers;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentRunMethod {
    Get,
    Post,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentRunMode {
    Sync,
    Async,
}

pub struct AgentRunRequest {
    pub method: AgentRunMethod,
    pub url: String,
    // the caller's Authorization value stays secret, so Debug names the headers without their values
    pub headers: Headers,
    pub body: Option<Vec<u8>>,
}

impl fmt::Debug for AgentRunRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<&str> = self.headers.keys().map(String::as_str).collect();
        f.debug_struct("AgentRunRequest")
            .field("method", &self.method)
            .field("url", &self.url)
            .field("header_names", &names)
            .field("body_length", &self.body.as_ref().map(Vec::len))
            .finish()
    }
}

#[async_trait]
pub trait AgentRunTransport: Send + Sync {
    async fn send(&self, request: AgentRunRequest) -> Result<UpstreamResponse, TransportError>;
}

// The handler builds its client with get_httpx_client or get_async_httpx_client, whose B173 clients stand behind this
pub trait AgentRunClientFactory: Send + Sync {
    fn sync_client(&self, ssl_verify: &Json) -> Box<dyn AgentRunTransport>;
    fn async_client(&self, ssl_verify: &Json) -> Box<dyn AgentRunTransport>;
}

// the polling loop waits on this instead of Python's time.sleep, so a test or the oracle supplies the clock
#[async_trait]
pub trait Sleeper: Send + Sync {
    async fn sleep(&self, duration: Duration);
}
