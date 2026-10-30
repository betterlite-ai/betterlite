mod callback;
mod discovery;
mod models;
mod oauth;

pub use callback::{CallbackError, LoopbackServer};
pub use discovery::{discover_cli_auth, origin};
pub use models::{
    CallbackCode, CallbackDenied, CallbackOutcome, CliAuthContract, PkceCredential, PkceFailure,
    RevocationUnavailable,
};
pub use oauth::{
    RedeemCodeRequest, RefreshCredentialRequest, RevocationUnavailableOrFailure, authorize_url,
    pkce_pair, redeem_code, refresh_credential, register_client, revoke_credential,
};

pub const CALLBACK_PATH: &str = "/callback";
pub const DISCOVERY_PATH: &str = "/.well-known/litellm-cli-auth";
pub const HTTP_TIMEOUT_SECONDS: u64 = 15;

pub trait Entropy {
    fn fill(&mut self, bytes: &mut [u8]) -> Result<(), PkceFailure>;
}

pub trait Clock {
    fn now_seconds(&self) -> i64;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        let headers = self.headers.iter();
        headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpBody {
    Json(Vec<u8>),
    Form(Vec<(String, String)>),
}

mod tests;
