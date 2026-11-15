use std::future::Future;

use crate::token_cache::error::CredentialCacheError;
use crate::token_cache::probe::AccessToken;

use super::super::client::PredictionClient;
use super::super::error::LegacyPredictionError;
use super::super::invocation::LegacyInvocation;
use super::streaming::stream_inner;
use super::{LegacyChunkStream, LegacyContext, StreamFailure};

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
