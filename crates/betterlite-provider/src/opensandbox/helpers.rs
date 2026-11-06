use std::string::FromUtf8Error;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::sandbox::SandboxOutputError;
use futures_util::stream::BoxStream;
use http::header::InvalidHeaderValue;
use http::{HeaderMap, Method};
use serde_json::Error as JsonError;
use thiserror::Error;
use tokio::time::{Instant, sleep};
use url::Url;

pub mod constants;
pub mod create;
pub mod poll;
pub mod request;
pub mod sse;
mod tests;

pub const fn check_status(status: u16) -> Result<(), OpenSandboxError> {
    match status {
        200..=299 => Ok(()),
        _ => Err(OpenSandboxError::HttpStatus { status }),
    }
}

fn output_error(error: SandboxOutputError<OpenSandboxError>) -> OpenSandboxError {
    match error {
        SandboxOutputError::Exceeded { limit } => OpenSandboxError::OutputTooLarge { limit },
        SandboxOutputError::Utf8 { source } => OpenSandboxError::OutputNotUtf8 { source },
        SandboxOutputError::Read { source } => source,
    }
}

#[derive(Debug, Error)]
pub enum OpenSandboxError {
    #[error("OpenSandbox api_base is required; pass api_base or set {variable}")]
    MissingApiBase { variable: &'static str },
    #[error("OpenSandbox sandbox {sandbox_id} entered {state}")]
    UnexpectedState { sandbox_id: String, state: String },
    #[error("OpenSandbox sandbox {sandbox_id} was not Running within {timeout:?}")]
    NotRunning {
        sandbox_id: String,
        timeout: Duration,
    },
    #[error("OpenSandbox execd endpoint for {sandbox_id} was not ready within {timeout:?}")]
    EndpointNotReady {
        sandbox_id: String,
        timeout: Duration,
        #[source]
        last: Option<Box<Self>>,
    },
    #[error("OpenSandbox did not return an execd endpoint for {sandbox_id}")]
    MissingEndpoint { sandbox_id: String },
    #[error("the OpenSandbox API answered with status {status}")]
    HttpStatus { status: u16 },
    #[error("the OpenSandbox response body is not the JSON the API documents")]
    Decode {
        #[source]
        source: JsonError,
    },
    #[error("the OpenSandbox request could not be encoded")]
    Encode {
        #[source]
        source: JsonError,
    },
    #[error("the sandbox URL is not valid")]
    Url {
        #[source]
        source: url::ParseError,
    },
    #[error("a credential holds a character a header cannot carry")]
    InvalidHeader {
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the sandbox output exceeded {limit} bytes; aborting to avoid unbounded memory use")]
    OutputTooLarge { limit: usize },
    #[error("the sandbox output is not valid UTF-8")]
    OutputNotUtf8 {
        #[source]
        source: FromUtf8Error,
    },
    #[error("the OpenSandbox request failed")]
    Transport {
        #[source]
        source: TransportError,
    },
    #[error("the sandbox response body failed")]
    Body {
        #[source]
        source: HttpError,
    },
}

#[derive(Debug, Clone)]
pub struct SandboxRequest {
    pub method: Method,
    pub url: Url,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
    pub timeout: Option<RequestTimeout>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestTimeout {
    pub connect: Option<Duration>,
    pub read: Option<Duration>,
    pub write: Option<Duration>,
    pub pool: Option<Duration>,
}

pub struct SandboxStream {
    pub status: u16,
    pub body: BoxStream<'static, Result<Vec<u8>, OpenSandboxError>>,
}

#[async_trait]
pub trait SandboxHttp: Send + Sync {
    async fn send(&self, request: SandboxRequest) -> Result<SandboxStream, OpenSandboxError>;
}

#[async_trait]
pub trait PollClock: Send + Sync {
    fn now(&self) -> Duration;
    async fn sleep(&self, duration: Duration);
}

pub struct MonotonicClock {
    pub origin: Instant,
}

#[async_trait]
impl PollClock for MonotonicClock {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }

    async fn sleep(&self, duration: Duration) {
        sleep(duration).await;
    }
}

pub struct OpenSandbox<Http, Environment, Clock> {
    pub http: Http,
    pub environment: Environment,
    pub clock: Clock,
}
