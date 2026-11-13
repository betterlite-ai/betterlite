use std::future::Future;

use secrecy::SecretString;

use crate::model_policy::capabilities::VertexCapabilityProvider;
use crate::token_cache::error::CredentialCacheError;
use crate::token_cache::probe::AccessToken;
use crate::token_refresh::ensure_access_token;

use super::super::error::PredictionError;
use super::LegacyContext;

pub(super) struct RpcScope {
    pub(super) token: SecretString,
    pub(super) project: String,
    pub(super) location: String,
}

pub(super) fn require_custom(context: &LegacyContext<'_, '_>) -> Result<(), PredictionError> {
    if context.project.is_some() && context.location.is_some() {
        return Ok(());
    }
    Err(PredictionError::CustomEndpointContext)
}

pub(super) async fn resolve_scope<Lookup>(
    context: &LegacyContext<'_, '_>,
    lookup: Lookup,
) -> Result<RpcScope, PredictionError>
where
    Lookup: Future<Output = Result<AccessToken, CredentialCacheError>> + Send,
{
    let access = ensure_access_token(VertexCapabilityProvider::VertexAi, lookup)
        .await
        .map_err(|source| PredictionError::Credentials { source })?;
    let project = match context.project {
        Some(project) => project.to_owned(),
        None => access.project.clone(),
    };
    let location = location_of(context)?;
    Ok(RpcScope {
        token: access.token,
        project,
        location,
    })
}

fn location_of(context: &LegacyContext<'_, '_>) -> Result<String, PredictionError> {
    context
        .location
        .map(str::to_owned)
        .ok_or(PredictionError::MissingField { field: "location" })
}
