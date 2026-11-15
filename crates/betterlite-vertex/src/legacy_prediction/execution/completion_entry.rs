use std::future::Future;

use crate::token_cache::error::CredentialCacheError;
use crate::token_cache::probe::AccessToken;

use super::super::client::PredictionClient;
use super::super::error::LegacyPredictionError;
use super::super::invocation::LegacyInvocation;
use super::super::projection::LegacyResult;
use super::LegacyContext;
use super::completion::complete_inner;

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
