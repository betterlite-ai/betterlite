use http::HeaderMap;
use secrecy::SecretString;

use super::error::SapEmbeddingError;
use crate::sap_auth::deployment::orchestration_headers;

pub const EMBEDDINGS_PATH: &str = "/v2/embeddings";

pub struct SapEmbeddingConfig {
    pub token: SecretString,
    pub resource_group: String,
    pub deployment_url: String,
}

#[derive(Debug)]
pub struct SapEmbeddingCall {
    pub url: String,
    pub headers: HeaderMap,
}

impl SapEmbeddingConfig {
    // LiteLLM's validate_environment and get_complete_url, which the embedding path runs back to back
    pub fn prepare(&self) -> Result<SapEmbeddingCall, SapEmbeddingError> {
        Ok(SapEmbeddingCall {
            url: format!(
                "{}{EMBEDDINGS_PATH}",
                self.deployment_url.trim_end_matches('/')
            ),
            headers: orchestration_headers(&self.token, &self.resource_group)?,
        })
    }
}
