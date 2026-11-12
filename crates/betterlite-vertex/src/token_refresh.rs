pub(crate) mod background;
mod reauth;
mod tests;

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, PoisonError};

use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use secrecy::SecretString;

use crate::model_policy::capabilities::VertexCapabilityProvider;
use crate::token_cache::CredentialCache;
use crate::token_cache::error::{CredentialCacheError, needs_reauthentication};
use crate::token_cache::key::CredentialKey;
use crate::token_cache::probe::{
    AccessToken, UsableToken, try_cached_token, try_usable_cached_token,
};
use crate::token_cache::record::{CacheRecord, CredentialSnapshot, unpack_cached_credentials};
use crate::token_cache::state::{TokenState, token_state};

use self::background::{await_background_refresh, schedule_background_refresh};
use self::reauth::reauthenticate_async;

// LiteLLM's get_access_token_async: fresh returns now, stale serves now and refreshes in the background, invalid blocks
pub async fn get_access_token_async(
    cache: &CredentialCache,
    credentials: Option<&ConfiguredCredentials>,
    project: Option<&str>,
) -> Result<AccessToken, CredentialCacheError> {
    let key = CredentialKey::new(credentials, project);
    if let Some(entry) = usable_cached(cache, &key, project) {
        if entry.state == TokenState::Stale {
            schedule_background_refresh(cache, entry.credential, key.clone(), entry.cached_project);
        }
        return Ok(AccessToken {
            token: entry.token,
            project: entry.project,
        });
    }
    // the lease covers the whole slow path so one caller refreshes and the rest read its result
    let _lease = cache.acquire_refresh_lease(&key).await?;
    locked_access_token(cache, credentials, project, &key).await
}

// the fast path reads the usable projection without taking the per-key lease
fn usable_cached(
    cache: &CredentialCache,
    key: &CredentialKey,
    project: Option<&str>,
) -> Option<UsableToken> {
    try_usable_cached_token(
        &cache.records.lock().unwrap_or_else(PoisonError::into_inner),
        key,
        project,
        cache.clock.now(),
    )
}

// LiteLLM's _ensure_access_token: gemini is Google AI Studio and reads no credentials at all
pub fn ensure_access_token<'borrow>(
    cache: &'borrow CredentialCache,
    credentials: Option<&'borrow ConfiguredCredentials>,
    project: Option<&'borrow str>,
    provider: VertexCapabilityProvider,
) -> Pin<Box<dyn Future<Output = Result<AccessToken, CredentialCacheError>> + Send + 'borrow>> {
    Box::pin(async move {
        if provider == VertexCapabilityProvider::Gemini {
            return Ok(gemini_access());
        }
        cache.get_access_token(credentials, project).await
    })
}

// LiteLLM's _ensure_access_token_async: the same mode selection over the coordinated lookup
pub async fn ensure_access_token_async(
    cache: &CredentialCache,
    credentials: Option<&ConfiguredCredentials>,
    project: Option<&str>,
    provider: VertexCapabilityProvider,
) -> Result<AccessToken, CredentialCacheError> {
    match provider {
        VertexCapabilityProvider::Gemini => Ok(gemini_access()),
        VertexCapabilityProvider::VertexAi | VertexCapabilityProvider::VertexAiBeta => {
            get_access_token_async(cache, credentials, project).await
        }
    }
}

// LiteLLM returns an empty token and project for Google AI Studio without reading any credentials
fn gemini_access() -> AccessToken {
    AccessToken {
        token: SecretString::from(""),
        project: String::new(),
    }
}

async fn locked_access_token(
    cache: &CredentialCache,
    credentials: Option<&ConfiguredCredentials>,
    project: Option<&str>,
    key: &CredentialKey,
) -> Result<AccessToken, CredentialCacheError> {
    let fresh = {
        let records = cache.records.lock().unwrap_or_else(PoisonError::into_inner);
        try_cached_token(&records, key, project, cache.clock.now())
    };
    if let Some(access) = fresh {
        return Ok(access);
    }
    let record = match cache.cached_record(key) {
        Some(record) => record,
        None => cache.load_and_cache(credentials, project, key).await?,
    };
    let (credential, credential_project) = unpack_record(&record)?;
    let resolved = project.map_or_else(
        || resolved_project_alias(cache, key, &credential, credential_project.as_deref()),
        |named| Some(named.to_owned()),
    );
    match token_state(&credential, cache.clock.now()) {
        TokenState::Fresh => validate(&credential, resolved),
        TokenState::Stale => {
            match stale_access(
                cache,
                key,
                resolved.as_deref(),
                &credential,
                credential_project.as_deref(),
            )? {
                Some(access) => Ok(access),
                None => invalid_access(cache, credentials, key, record, resolved).await,
            }
        }
        TokenState::Invalid => invalid_access(cache, credentials, key, record, resolved).await,
    }
}

// LiteLLM's slow path always overwrites the resolved alias; the synchronous lookup only fills an absent one
fn resolved_project_alias(
    cache: &CredentialCache,
    key: &CredentialKey,
    credential: &Arc<CredentialSnapshot>,
    credential_project: Option<&str>,
) -> Option<String> {
    let project = credential_project?;
    cache.store_record(
        CredentialKey {
            credential: key.credential.clone(),
            project: Some(project.to_owned()),
        },
        CacheRecord::Paired {
            credential: Arc::clone(credential),
            project: Some(project.to_owned()),
        },
    );
    Some(project.to_owned())
}

fn stale_access(
    cache: &CredentialCache,
    key: &CredentialKey,
    resolved: Option<&str>,
    credential: &Arc<CredentialSnapshot>,
    credential_project: Option<&str>,
) -> Result<Option<AccessToken>, CredentialCacheError> {
    let project = resolved.ok_or(CredentialCacheError::MissingProject)?;
    let Some(token) = credential.token.clone() else {
        return Ok(None);
    };
    schedule_background_refresh(
        cache,
        Arc::clone(credential),
        key.clone(),
        credential_project.map(str::to_owned),
    );
    Ok(Some(AccessToken {
        token,
        project: project.to_owned(),
    }))
}

// LiteLLM's INVALID branch: drain a background refresh first, then block on a refresh of this handle
async fn invalid_access(
    cache: &CredentialCache,
    credentials: Option<&ConfiguredCredentials>,
    key: &CredentialKey,
    record: CacheRecord,
    resolved: Option<String>,
) -> Result<AccessToken, CredentialCacheError> {
    await_background_refresh(cache, key).await;
    let fresh = {
        let records = cache.records.lock().unwrap_or_else(PoisonError::into_inner);
        try_cached_token(&records, key, resolved.as_deref(), cache.clock.now())
    };
    if let Some(access) = fresh {
        return Ok(access);
    }
    let (credential, credential_project) = unpack_record(&record)?;
    match refresh_credential(cache, key, &credential, credential_project).await {
        Ok(refreshed) => validate(&refreshed, resolved),
        Err(error) if needs_reauthentication(&error) => {
            reauthenticate_async(cache, credentials, resolved, key, error).await
        }
        Err(error) => Err(error),
    }
}

async fn refresh_credential(
    cache: &CredentialCache,
    key: &CredentialKey,
    credential: &CredentialSnapshot,
    credential_project: Option<String>,
) -> Result<Arc<CredentialSnapshot>, CredentialCacheError> {
    let refreshed = Arc::new(cache.executor.refresh(credential).await?);
    // LiteLLM keeps the project the key resolved while replacing the credentials sitting at that key
    cache.store_record(
        key.clone(),
        CacheRecord::Paired {
            credential: Arc::clone(&refreshed),
            project: credential_project,
        },
    );
    Ok(refreshed)
}

fn unpack_record(
    record: &CacheRecord,
) -> Result<(Arc<CredentialSnapshot>, Option<String>), CredentialCacheError> {
    let (cached, project) = unpack_cached_credentials(Some(record));
    let credential = cached.ok_or(CredentialCacheError::MissingCredentials)?;
    Ok((Arc::clone(credential), project.map(str::to_owned)))
}

fn validate(
    credential: &CredentialSnapshot,
    resolved: Option<String>,
) -> Result<AccessToken, CredentialCacheError> {
    let token = credential
        .token
        .clone()
        .ok_or(CredentialCacheError::MissingToken)?;
    let project = resolved.ok_or(CredentialCacheError::MissingProject)?;
    Ok(AccessToken { token, project })
}
