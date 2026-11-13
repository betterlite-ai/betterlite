use std::io::Error as IoError;

use secrecy::SecretString;

use crate::pkce_login::HttpResponse;

pub trait PollTransport {
    fn get(
        &self,
        url: &str,
        headers: &[(String, SecretString)],
        timeout_seconds: u64,
    ) -> Result<HttpResponse, IoError>;

    fn post(&self, url: &str, timeout_seconds: u64) -> Result<HttpResponse, IoError>;
}
