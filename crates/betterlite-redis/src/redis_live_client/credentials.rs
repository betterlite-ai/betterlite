use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use futures_util::Stream;
use futures_util::stream::unfold;
use redis::auth::StreamingCredentialsProvider;
use redis::{BasicAuth, ErrorKind, RedisError, RedisResult};
use secrecy::{ExposeSecret as _, SecretString};
use tokio::runtime::Handle;
use tokio::time::sleep;

use super::LiveClientError;
use crate::redis_client_factories::cluster::ClusterClient as ClusterRecord;
use crate::redis_client_factories::fields::ConnectionFields;
use crate::redis_client_factories::url::UrlClient;

pub struct RedisCredentials {
    pub username: Option<String>,
    pub password: SecretString,
}

#[derive(Debug, thiserror::Error)]
pub enum RedisCredentialError {
    #[error("the redis credential source could not fetch a credential")]
    Fetch(#[source] io::Error),
}

pub(crate) trait CredentialNeeds {
    fn needs_source(&self) -> bool;
}

impl CredentialNeeds for ConnectionFields {
    fn needs_source(&self) -> bool {
        self.credential_provider || self.credential_provider_settings.is_some()
    }
}

impl CredentialNeeds for UrlClient {
    fn needs_source(&self) -> bool {
        self.credential_provider || self.credential_provider_settings.is_some()
    }
}

impl CredentialNeeds for ClusterRecord {
    fn needs_source(&self) -> bool {
        self.credential_provider || self.credential_provider_settings.is_some()
    }
}

pub trait RedisCredentialSource: Send + Sync + 'static {
    fn fetch(&self) -> Result<RedisCredentials, RedisCredentialError>;
    fn refresh_after(&self) -> Duration;
}

pub(crate) fn required(
    injected: Option<&SourceProvider>,
    needed: bool,
) -> Result<Option<SourceProvider>, LiveClientError> {
    if !needed {
        return Ok(None);
    }
    injected
        .cloned()
        .map(Some)
        .ok_or(LiveClientError::CredentialSourceMissing)
}

pub(crate) fn fetch(
    injected: Option<&SourceProvider>,
) -> Result<Option<RedisCredentials>, LiveClientError> {
    let Some(provider) = injected else {
        return Ok(None);
    };
    provider
        .source
        .fetch()
        .map(Some)
        .map_err(LiveClientError::Credential)
}

#[derive(Clone)]
pub(crate) struct SourceProvider {
    pub(crate) source: Arc<dyn RedisCredentialSource>,
}

impl From<Arc<dyn RedisCredentialSource>> for SourceProvider {
    fn from(source: Arc<dyn RedisCredentialSource>) -> Self {
        Self { source }
    }
}

impl StreamingCredentialsProvider for SourceProvider {
    fn subscribe(&self) -> Pin<Box<dyn Stream<Item = RedisResult<BasicAuth>> + Send + 'static>> {
        let provider = Arc::clone(&self.source);
        Box::pin(unfold(true, move |first| {
            let source = Arc::clone(&provider);
            async move { Some((next_credentials(source, first).await, false)) }
        }))
    }
}

async fn next_credentials(
    source: Arc<dyn RedisCredentialSource>,
    first: bool,
) -> RedisResult<BasicAuth> {
    if !first {
        sleep(source.refresh_after()).await;
    }
    let handle = Handle::current();
    let joined = handle.spawn_blocking(move || source.fetch()).await;
    match joined {
        Ok(Ok(credentials)) => Ok(BasicAuth::new(
            credentials.username.unwrap_or_else(|| "default".to_owned()),
            credentials.password.expose_secret().to_owned(),
        )),
        Ok(Err(_)) | Err(_) => Err(RedisError::from((
            ErrorKind::AuthenticationFailed,
            "the redis credential source could not fetch a credential",
        ))),
    }
}
