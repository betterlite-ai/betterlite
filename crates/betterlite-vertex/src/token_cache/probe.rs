use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::SystemTime;

use secrecy::SecretString;

use super::key::CredentialKey;
use super::record::{CacheRecord, CredentialSnapshot, unpack_cached_credentials};
use super::state::{TokenState, token_state};

#[derive(Debug, Clone)]
pub struct AccessToken {
    pub token: SecretString,
    pub project: String,
}

#[derive(Debug, Clone)]
pub struct UsableToken {
    pub token: SecretString,
    pub project: String,
    pub state: TokenState,
    pub credential: Arc<CredentialSnapshot>,
    pub cached_project: Option<String>,
}

// LiteLLM's _try_get_cached_token: only a fresh token and a project we can name make a hit
#[must_use]
pub fn try_cached_token(
    records: &BTreeMap<CredentialKey, CacheRecord>,
    key: &CredentialKey,
    requested_project: Option<&str>,
    now: SystemTime,
) -> Option<AccessToken> {
    let (cached, cached_project) = unpack_cached_credentials(records.get(key));
    let credential = cached?;
    if token_state(credential, now) != TokenState::Fresh {
        return None;
    }
    let token = credential.token.clone()?;
    let project = requested_project.or(cached_project)?;
    Some(AccessToken {
        token,
        project: project.to_owned(),
    })
}

// LiteLLM's _try_get_usable_cached_token: a stale token still serves the request while it refreshes
#[must_use]
pub fn try_usable_cached_token(
    records: &BTreeMap<CredentialKey, CacheRecord>,
    key: &CredentialKey,
    requested_project: Option<&str>,
    now: SystemTime,
) -> Option<UsableToken> {
    let (cached, cached_project) = unpack_cached_credentials(records.get(key));
    let credential = cached?;
    let state = token_state(credential, now);
    if !matches!(state, TokenState::Fresh | TokenState::Stale) {
        return None;
    }
    let token = credential.token.clone()?;
    let project = requested_project.or(cached_project)?;
    Some(UsableToken {
        token,
        project: project.to_owned(),
        state,
        credential: Arc::clone(credential),
        cached_project: cached_project.map(str::to_owned),
    })
}
