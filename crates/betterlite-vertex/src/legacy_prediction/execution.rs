use std::future::Future;

use futures_util::stream::BoxStream;

use crate::token_cache::error::CredentialCacheError;
use crate::token_cache::probe::AccessToken;

use super::client::PredictionClient;
use super::error::{LegacyPredictionError, PredictionError};
use super::invocation::LegacyInvocation;
use super::projection::{LegacyResult, ProjectionServices};
use super::records::LegacyChunk;

use self::completion::complete_inner;
use self::streaming::stream_inner;

mod completion;
mod scope;
mod streaming;

pub type LegacyChunkStream = BoxStream<'static, Result<LegacyChunk, LegacyPredictionError>>;

pub struct LegacyContext<'a, 'b> {
    pub project: Option<&'a str>,
    pub location: Option<&'a str>,
    pub model_is_language: bool,
    pub services: &'a ProjectionServices<'b>,
}

pub(super) enum StreamFailure {
    Unable,
    Prediction(PredictionError),
}

impl From<PredictionError> for StreamFailure {
    fn from(source: PredictionError) -> Self {
        Self::Prediction(source)
    }
}

pub async fn complete<C, Lookup>(
    client: &C,
    invocation: LegacyInvocation,
    context: &LegacyContext<'_, '_>,
    lookup: Lookup,
) -> Result<LegacyResult, LegacyPredictionError>
where
    C: PredictionClient,
    Lookup: Future<Output = Result<AccessToken, CredentialCacheError>> + Send,
{
    complete_inner(client, invocation, context, lookup)
        .await
        .map_err(|source| LegacyPredictionError::Completion { source })
}

pub async fn stream<C, Lookup>(
    client: &C,
    invocation: LegacyInvocation,
    context: &LegacyContext<'_, '_>,
    lookup: Lookup,
) -> Result<LegacyChunkStream, LegacyPredictionError>
where
    C: PredictionClient,
    Lookup: Future<Output = Result<AccessToken, CredentialCacheError>> + Send,
{
    match stream_inner(client, invocation, context, lookup).await {
        Ok(stream) => Ok(stream),
        Err(StreamFailure::Unable) => Err(LegacyPredictionError::UnableToGenerate),
        Err(StreamFailure::Prediction(source)) => Err(LegacyPredictionError::Streaming { source }),
    }
}
