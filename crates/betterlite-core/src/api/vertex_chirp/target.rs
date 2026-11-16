use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use secrecy::SecretString;

pub type AccessTokenFuture<'target> =
    Pin<Box<dyn Future<Output = Result<SecretString, AccessTokenError>> + Send + 'target>>;

pub trait AccessTokenResolver: Send + Sync {
    fn resolve_access_token(&self) -> AccessTokenFuture<'_>;
}

#[derive(Debug, thiserror::Error)]
#[error("the access token could not be resolved")]
pub struct AccessTokenError;

pub struct SpeechStreamingTarget {
    pub api_endpoint: String,
    pub recognizer: String,
    pub token: Arc<dyn AccessTokenResolver>,
}
