use std::future::Future;
use std::io::Error as IoError;
use std::pin::Pin;
use std::str::{Utf8Error, from_utf8};
use std::time::Duration;

use secrecy::SecretString;
use thiserror::Error;

use crate::cyberark::transport::ClientCertificate;

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);
pub const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
    Delete,
}

#[derive(Debug, Error)]
pub enum HttpError {
    #[error("the HTTP request timed out")]
    Timeout,
    #[error("the HTTP request failed")]
    Transport(#[source] IoError),
    #[error("the HTTP response is longer than {limit} bytes")]
    ResponseTooLarge { limit: usize },
    #[error("the HTTP response body is not valid UTF-8")]
    BodyNotUtf8(#[source] Utf8Error),
}

// No Debug: the header values and the body can hold a secret
#[derive(Clone)]
pub struct HttpRequest {
    pub method: HttpMethod,
    pub url: String,
    pub headers: Vec<(&'static str, String)>,
    pub body: Option<String>,
    pub timeout: Duration,
    pub follow_redirects: bool,
    pub verify_tls: bool,
    pub certificate: Option<ClientCertificate>,
    pub max_response_bytes: usize,
}

#[derive(Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

pub struct Reply {
    pub status: u16,
    pub body: SecretString,
}

pub type ExchangeFuture<'request> =
    Pin<Box<dyn Future<Output = Result<HttpResponse, HttpError>> + Send + 'request>>;

// The socket client lives above this crate: betterlite-http owns it and the wiring injects it here
pub trait HttpExchange: Send + Sync {
    fn send(&self, request: HttpRequest) -> ExchangeFuture<'_>;

    fn send_blocking(&self, request: HttpRequest) -> Result<HttpResponse, HttpError>;
}

pub(super) fn into_reply(response: HttpResponse, limit: usize) -> Result<Reply, HttpError> {
    let HttpResponse { status, body } = response;
    if body.len() > limit {
        return Err(HttpError::ResponseTooLarge { limit });
    }
    let text = from_utf8(&body)
        .map(SecretString::from)
        .map_err(HttpError::BodyNotUtf8)?;
    Ok(Reply { status, body: text })
}
