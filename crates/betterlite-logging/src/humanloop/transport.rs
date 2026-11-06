use std::error::Error;
use std::future::Future;

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
    type Error: Error + Send + Sync + 'static;

    fn get(
        &self,
        fetch: &PromptFetch,
    ) -> impl Future<Output = Result<TransportResponse, Self::Error>> + Send;
}
