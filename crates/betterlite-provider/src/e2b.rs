use std::collections::BTreeMap;
use std::string::FromUtf8Error;

use async_trait::async_trait;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::sandbox::ContainerHandle;
use futures_util::stream::BoxStream;
use http::header::InvalidHeaderValue;
use http::{HeaderMap, HeaderValue, Method};
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;
use url::Url;

pub const E2B_API_BASE: &str = "https://api.e2b.app";
pub const E2B_DEFAULT_TEMPLATE: &str = "code-interpreter-v1";
pub const E2B_DEFAULT_DOMAIN: &str = "e2b.app";
pub const JUPYTER_PORT: u16 = 49999;
pub const DEFAULT_SANDBOX_TIMEOUT: i64 = 300;

mod operations;
pub mod parse;
mod tests;
pub mod transport;

#[derive(Debug, Error)]
pub enum E2bError {
    #[error("E2B_API_KEY is not set; pass api_key or set the environment variable")]
    MissingApiKey,
    #[error(
        "the sandbox has no access token; pass the ContainerHandle that acreate_sandbox returned, not the sandbox id"
    )]
    MissingAccessToken,
    #[error("the E2B API answered with status {status}")]
    HttpStatus { status: u16 },
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
    #[error("the sandbox request could not be encoded")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the create response is not the JSON acreate_sandbox expects")]
    CreateResponse {
        #[source]
        source: serde_json::Error,
    },
    #[error("the sandbox response body is larger than {limit} bytes")]
    ResponseTooLarge { limit: usize },
    #[error("the sandbox output exceeded {limit} bytes; aborting to avoid unbounded memory use")]
    OutputTooLarge { limit: usize },
    #[error("the sandbox output is not valid UTF-8")]
    OutputNotUtf8 {
        #[source]
        source: FromUtf8Error,
    },
    #[error("the E2B request failed")]
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
}

pub struct SandboxStream {
    pub status: u16,
    pub body: BoxStream<'static, Result<Vec<u8>, E2bError>>,
}

#[async_trait]
pub trait SandboxHttp: Send + Sync {
    async fn send(&self, request: SandboxRequest) -> Result<SandboxStream, E2bError>;
}

pub struct E2bSandbox<Http, Environment> {
    pub http: Http,
    pub environment: Environment,
}

pub struct CreateSandbox<'request> {
    pub template: Option<&'request str>,
    pub timeout: Option<i64>,
    pub allow_internet_access: Option<bool>,
    pub api_key: Option<&'request SecretString>,
    pub api_base: Option<&'request str>,
}

pub struct RunCode<'request> {
    pub container: &'request ContainerHandle,
    pub code: &'request str,
    pub env_vars: Option<&'request BTreeMap<String, String>>,
}

#[derive(Clone, Copy)]
pub enum SandboxRef<'request> {
    Handle(&'request ContainerHandle),
    Id(&'request str),
}

pub struct DeleteSandbox<'request> {
    pub container: SandboxRef<'request>,
    pub api_key: Option<&'request SecretString>,
    pub api_base: Option<&'request str>,
}

pub(crate) const fn check_status(status: u16) -> Result<(), E2bError> {
    if status >= 400 {
        return Err(E2bError::HttpStatus { status });
    }
    Ok(())
}

pub(crate) fn secret_header_value(secret: &SecretString) -> Result<HeaderValue, E2bError> {
    let mut value = HeaderValue::try_from(secret.expose_secret())
        .map_err(|source| E2bError::InvalidHeader { source })?;
    value.set_sensitive(true);
    Ok(value)
}
