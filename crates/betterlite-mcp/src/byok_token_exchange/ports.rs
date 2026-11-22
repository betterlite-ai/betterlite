mod tests;

use std::future::Future;

use secrecy::SecretString;
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct AuthCodeRecord {
    pub api_key: SecretString,
    pub server_id: String,
    pub code_challenge: String,
    pub redirect_uri: String,
    pub client_id: String,
    pub user_id: String,
    pub expires_at: f64,
}

pub trait AuthCodeLookup: Send + Sync {
    fn purge_expired(&self, now: f64);

    fn get(&self, code: &str) -> Option<AuthCodeRecord>;

    fn remove(&self, code: &str);
}

// The sink's causes live in crates this port cannot name, so the implementing crate reports them as text; LiteLLM logs exc the same way.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SinkError {
    #[error("the credential could not be stored: {message}")]
    Store { message: String },
    #[error("the credential cache could not be invalidated: {message}")]
    Invalidate { message: String },
}

pub trait ByokCredentialSink: Send + Sync {
    fn store_credential(
        &self,
        user_id: &str,
        server_id: &str,
        credential: &SecretString,
    ) -> impl Future<Output = Result<(), SinkError>> + Send;

    fn invalidate_cache(
        &self,
        user_id: &str,
        server_id: &str,
    ) -> impl Future<Output = Result<(), SinkError>> + Send;
}
