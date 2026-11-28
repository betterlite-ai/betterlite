use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_http::gigachat_tokens::error::GigachatAuthError;
use betterlite_http::gigachat_tokens::request::GigachatTokenRequest;
use betterlite_http::gigachat_tokens::tokens::GigachatTokens;
use secrecy::SecretString;

use super::secret;

pub(crate) struct FakeTokens {
    requests: Mutex<Vec<GigachatTokenRequest>>,
}

impl FakeTokens {
    pub(crate) const fn new() -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn request(&self) -> GigachatTokenRequest {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .first()
            .cloned()
            .expect("the bearer asked the service")
    }
}

#[async_trait]
impl GigachatTokens for FakeTokens {
    async fn access_token(
        &self,
        request: &GigachatTokenRequest,
    ) -> Result<SecretString, GigachatAuthError> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.clone());
        Ok(secret("token-1"))
    }
}

pub(crate) struct FailingTokens;

#[async_trait]
impl GigachatTokens for FailingTokens {
    async fn access_token(
        &self,
        _request: &GigachatTokenRequest,
    ) -> Result<SecretString, GigachatAuthError> {
        Err(GigachatAuthError::MissingCredentials)
    }
}
