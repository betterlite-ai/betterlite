use std::error::Error;

use secrecy::SecretString;

#[derive(Debug)]
pub struct PromptFetch {
    pub url: String,
    pub headers: Vec<(&'static str, SecretString)>,
}

#[derive(Debug)]
pub struct TransportResponse {
    pub status: u16,
    pub body: String,
}

pub trait PromptTransport {
    type Error: Error + 'static;

    fn get(&self, fetch: &PromptFetch) -> Result<TransportResponse, Self::Error>;
}
