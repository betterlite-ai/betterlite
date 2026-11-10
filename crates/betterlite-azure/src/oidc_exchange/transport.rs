use std::io;

use secrecy::SecretString;

#[derive(Debug, Clone)]
pub struct HttpReply {
    pub status: u16,
    pub body: SecretString,
}

// the outbound pool is injected, so a fixture drives every protocol case without leaving the host
pub trait Transport: Send + Sync {
    fn post_form(&self, url: &str, form: &[(String, SecretString)]) -> io::Result<HttpReply>;
}

pub trait Clock: Send + Sync {
    fn now_unix_seconds(&self) -> f64;
}
