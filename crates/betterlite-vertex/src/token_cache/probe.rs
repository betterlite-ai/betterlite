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

struct CachedMatch<'records> {
    credential: &'records Arc<CredentialSnapshot>,
    project: String,
    cached_project: Option<String>,
}

// the cached credential and the project LiteLLM can name for it, before either probe checks freshness
fn cached_credential<'records>(
    records: &'records BTreeMap<CredentialKey, CacheRecord>,
    key: &CredentialKey,
    requested_project: Option<&str>,
) -> Option<CachedMatch<'records>> {
    let (cached, cached_project) = unpack_cached_credentials(records.get(key));
    let credential = cached?;
    let project = requested_project.or(cached_project)?;
    Some(CachedMatch {
        credential,
        project: project.to_owned(),
        cached_project: cached_project.map(str::to_owned),
    })
}

// LiteLLM's _try_get_cached_token: the usable projection with a fresh token and nothing else
#[must_use]
pub fn try_cached_token(
    records: &BTreeMap<CredentialKey, CacheRecord>,
    key: &CredentialKey,
    requested_project: Option<&str>,
    now: SystemTime,
) -> Option<AccessToken> {
    match try_usable_cached_token(records, key, requested_project, now) {
        Some(usable) if usable.state == TokenState::Fresh => Some(AccessToken {
            token: usable.token,
            project: usable.project,
        }),
        _ => None,
    }
}

// LiteLLM's _try_get_usable_cached_token: a stale token still serves the request while it refreshes
#[must_use]
pub fn try_usable_cached_token(
    records: &BTreeMap<CredentialKey, CacheRecord>,
    key: &CredentialKey,
    requested_project: Option<&str>,
    now: SystemTime,
) -> Option<UsableToken> {
    let matched = cached_credential(records, key, requested_project)?;
    let state = token_state(matched.credential, now);
    if !matches!(state, TokenState::Fresh | TokenState::Stale) {
        return None;
    }
    let token = matched.credential.token.clone()?;
    Some(UsableToken {
        token,
        project: matched.project,
        state,
        credential: Arc::clone(matched.credential),
        cached_project: matched.cached_project,
    })
}
