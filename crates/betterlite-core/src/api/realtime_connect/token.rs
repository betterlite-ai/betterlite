use std::error::Error;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use betterlite_models::decisions::Json;
use secrecy::SecretString;
use thiserror::Error;

use crate::api::vertex_chirp::target::{
    AccessTokenError, AccessTokenFuture, AccessTokenResolver,
};

use super::RealtimeConnectError;

// what one credential resolution hands back: the access token and the project it resolved
pub type ResolvedVertexToken = (SecretString, String);

pub type VertexTokenFuture<'source> = Pin<
    Box<dyn Future<Output = Result<ResolvedVertexToken, VertexTokenError>> + Send + 'source>,
>;

// B1303's credential resolution behind the shape _ensure_access_token_async takes
pub trait VertexAccessTokenSource: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn resolve<'source>(
        &'source self,
        credentials: Option<&'source Json>,
        project_id: Option<&'source str>,
    ) -> VertexTokenFuture<'source>;
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct VertexTokenError {
    pub message: String,
}

// asyncio.wait_for: a zero or negative bound expires without polling, and an infinite one never expires
pub async fn resolve_vertex_access_token_bounded<Source>(
    source: &Source,
    credentials: Option<&Json>,
    project_id: Option<&str>,
    timeout_seconds: f64,
) -> Result<ResolvedVertexToken, RealtimeConnectError<Source::Error>>
where
    Source: VertexAccessTokenSource + ?Sized,
{
    if timeout_seconds.is_nan() || timeout_seconds <= 0.0_f64 {
        return Err(vertex_token_timeout(timeout_seconds));
    }
    let resolved = if timeout_seconds.is_infinite() {
        source.resolve(credentials, project_id).await
    } else {
        let limit = Duration::try_from_secs_f64(timeout_seconds)
            .map_err(|_invalid| vertex_token_timeout(timeout_seconds))?;
        tokio::time::timeout(limit, source.resolve(credentials, project_id))
            .await
            .map_err(|_elapsed| vertex_token_timeout(timeout_seconds))?
    };
    resolved.map_err(RealtimeConnectError::Token)
}

fn vertex_token_timeout<SourceError>(timeout_seconds: f64) -> RealtimeConnectError<SourceError> {
    RealtimeConnectError::VertexTokenTimeout { timeout_seconds }
}

// the nested resolve_vertex_access_token closure: the same credentials and the project the first call resolved
pub struct VertexTokenRefresh<SourceError> {
    source: Arc<dyn VertexAccessTokenSource<Error = SourceError>>,
    credentials: Option<Json>,
    project: String,
    timeout_seconds: f64,
}

impl<SourceError> VertexTokenRefresh<SourceError>
where
    SourceError: Error + Send + Sync + 'static,
{
    pub fn new(
        source: Arc<dyn VertexAccessTokenSource<Error = SourceError>>,
        credentials: Option<Json>,
        project: String,
        timeout_seconds: f64,
    ) -> Self {
        Self {
            source,
            credentials,
            project,
            timeout_seconds,
        }
    }
}

impl<SourceError> AccessTokenResolver for VertexTokenRefresh<SourceError>
where
    SourceError: Error + Send + Sync + 'static,
{
    // a refresh that fails says only that the token could not be resolved; the caller reconnects
    fn resolve_access_token(&self) -> AccessTokenFuture<'_> {
        Box::pin(async move {
            let (access_token, _project) = resolve_vertex_access_token_bounded(
                self.source.as_ref(),
                self.credentials.as_ref(),
                Some(&self.project),
                self.timeout_seconds,
            )
            .await
            .map_err(|_error| AccessTokenError)?;
            Ok(access_token)
        })
    }
}
