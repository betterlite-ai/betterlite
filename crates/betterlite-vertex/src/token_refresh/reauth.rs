use std::sync::Arc;

use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;

use crate::token_cache::CredentialCache;
use crate::token_cache::error::CredentialCacheError;
use crate::token_cache::key::CredentialKey;
use crate::token_cache::probe::AccessToken;
use crate::token_cache::record::{CacheRecord, unpack_cached_credentials};

use super::validate;

// LiteLLM's _handle_reauthentication_async: the retry stays inside the per-key lock and the first failure survives
pub(super) async fn reauthenticate_async(
    cache: &CredentialCache,
    credentials: Option<&ConfiguredCredentials>,
    project: Option<String>,
    key: &CredentialKey,
    original: CredentialCacheError,
) -> Result<AccessToken, CredentialCacheError> {
    cache.forget_record(key);
    match load_resolved(cache, credentials, project, key).await {
        Ok(access) => Ok(access),
        Err(_retry) => Err(original),
    }
}

async fn load_resolved(
    cache: &CredentialCache,
    credentials: Option<&ConfiguredCredentials>,
    project: Option<String>,
    key: &CredentialKey,
) -> Result<AccessToken, CredentialCacheError> {
    let record = cache
        .load_and_cache(credentials, project.as_deref(), key)
        .await?;
    let (credential, credential_project) = unpack_cached_credentials(Some(&record));
    let Some(loaded) = credential else {
        return Err(CredentialCacheError::MissingCredentials);
    };
    let mut resolved = project;
    if resolved.is_none()
        && let Some(alias) = credential_project
    {
        // unlike the synchronous lookup, async reauthentication overwrites any entry already at the resolved key
        cache.store_record(
            CredentialKey {
                credential: key.credential.clone(),
                project: Some(alias.to_owned()),
            },
            CacheRecord::Paired {
                credential: Arc::clone(loaded),
                project: Some(alias.to_owned()),
            },
        );
        resolved = Some(alias.to_owned());
    }
    validate(loaded, resolved)
}
