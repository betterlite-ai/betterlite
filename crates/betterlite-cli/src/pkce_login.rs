pub mod callback;
pub mod discovery;
pub mod models;
pub mod oauth;
pub mod revoke;
mod wire;

use models::PkceFailure;
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpBody {
    Json(Vec<u8>),
    Form(Vec<(String, String)>),
}

mod tests;
