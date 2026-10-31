pub mod callback;
pub mod discovery;
pub mod loopback;
pub mod models;
pub mod oauth;
pub mod request;
pub mod revoke;
pub mod wire;

use models::PkceFailure;
use secrecy::SecretString;
use std::fmt;
use std::time::Instant;

pub const CALLBACK_PATH: &str = "/callback";
pub const DISCOVERY_PATH: &str = "/.well-known/litellm-cli-auth";
pub const HTTP_TIMEOUT_SECONDS: u64 = 15;

pub trait Entropy {
    fn fill(&mut self, bytes: &mut [u8]) -> Result<(), PkceFailure>;
}

pub trait Clock {
    fn now(&self) -> Instant;
}

// Wall-clock seconds since the Unix epoch, injected so a caller can sample it after the response arrives instead of depending on the host clock.
pub trait WallClock {
    fn now_seconds(&self) -> i64;
}

#[derive(Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl fmt::Debug for HttpResponse {
    // A derived Debug would print the response body verbatim, and that body carries access and refresh tokens.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpResponse")
            .field("status", &self.status)
            .field("headers", &self.headers)
            .field("body", &format_args!("[{} bytes]", self.body.len()))
            .finish()
    }
}

pub trait Http {
    fn get(&self, url: &str, timeout_seconds: u64) -> Result<HttpResponse, PkceFailure>;

    fn post(
        &self,
        url: &str,
        body: HttpBody,
        timeout_seconds: u64,
        follow_redirects: bool,
    ) -> Result<HttpResponse, PkceFailure>;
}

#[derive(Clone)]
pub enum HttpBody {
    Json(Vec<u8>),
    Form(Vec<(String, SecretString)>),
}

impl fmt::Debug for HttpBody {
    // Form fields carry authorization codes, PKCE verifiers and refresh tokens that a derived Debug would print.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(bytes) => f
                .debug_tuple("Json")
                .field(&format_args!("[{} bytes]", bytes.len()))
                .finish(),
            Self::Form(fields) => f
                .debug_tuple("Form")
                .field(&format_args!("[{} fields]", fields.len()))
                .finish(),
        }
    }
}

mod tests;
