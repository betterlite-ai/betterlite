use betterlite_config::sso_pkce::PkceSettings;
use secrecy::SecretString;

use super::verifier_cache::{CacheRead, PkceCacheEntry, PkceCacheKind, PkceCaches, VerifierCache};
use super::{PKCE_VERIFIER_CACHE_PREFIX, PkceCacheShape, PkceError};

#[derive(Debug)]
pub struct PkceVerifier {
    pub cache_key: String,
    pub code_verifier: SecretString,
}

#[derive(Debug)]
pub struct TokenExchangeParameters {
    pub include_client_id: bool,
    pub verifier: Option<PkceVerifier>,
}

pub async fn token_exchange_parameters<Redis, User>(
    callback_state: Option<&str>,
    include_client_id: bool,
    settings: &PkceSettings,
    caches: &PkceCaches<'_, Redis, User>,
) -> Result<TokenExchangeParameters, PkceError>
where
    Redis: VerifierCache,
    User: VerifierCache,
{
    let mut parameters = TokenExchangeParameters {
        include_client_id,
        verifier: None,
    };
    let Some(state) = callback_state.filter(|value| !value.is_empty()) else {
        if settings.use_pkce {
            tracing::warn!(
                "PKCE is enabled but the callback carried no state parameter, so the verifier cannot be retrieved and the token exchange continues without one"
            );
        }
        return Ok(parameters);
    };
    if !settings.use_pkce {
        return Ok(parameters);
    }
    let cache_key = format!("{PKCE_VERIFIER_CACHE_PREFIX}{state}");
    match cache_lookup(caches, &cache_key, state).await {
        CacheLookup::Found(verifier) => {
            tracing::debug!("PKCE code verifier retrieved from cache");
            parameters.verifier = Some(PkceVerifier {
                cache_key,
                code_verifier: verifier,
            });
        }
        CacheLookup::Missing(kind) if settings.strict_cache_miss => {
            strict_miss(caches, &cache_key, state, kind).await?;
        }
        CacheLookup::Missing(kind) => permissive_miss(caches, &cache_key, state, kind).await,
    }
    Ok(parameters)
}

enum CacheLookup {
    Found(SecretString),
    Missing(MissingKind),
}

#[derive(Clone, Copy)]
enum MissingKind {
    Absent,
    Empty,
    Unexpected(PkceCacheShape),
}

async fn cache_lookup<Redis, User>(
    caches: &PkceCaches<'_, Redis, User>,
    cache_key: &str,
    state: &str,
) -> CacheLookup
where
    Redis: VerifierCache,
    User: VerifierCache,
{
    let CacheRead::Found(entry) = caches.read_verifier(cache_key).await else {
        return CacheLookup::Missing(MissingKind::Absent);
    };
    match entry {
        PkceCacheEntry::Verifier(verifier) => CacheLookup::Found(verifier),
        PkceCacheEntry::LegacyVerifier(verifier) => {
            tracing::warn!(
                "Retrieved the PKCE code verifier in the legacy plain-string format; future storage uses the object format"
            );
            CacheLookup::Found(verifier)
        }
        PkceCacheEntry::EmptyVerifier => {
            tracing::warn!(
                state,
                "the PKCE verifier cache entry has an empty or null code_verifier value; treating it as a cache miss"
            );
            CacheLookup::Missing(MissingKind::Empty)
        }
        PkceCacheEntry::UnexpectedShape(shape) => {
            tracing::debug!(
                state,
                ?shape,
                "the PKCE verifier cache entry has an unexpected shape; treating it as a cache miss"
            );
            CacheLookup::Missing(MissingKind::Unexpected(shape))
        }
    }
}

async fn strict_miss<Redis, User>(
    caches: &PkceCaches<'_, Redis, User>,
    cache_key: &str,
    state: &str,
    kind: MissingKind,
) -> Result<(), PkceError>
where
    Redis: VerifierCache,
    User: VerifierCache,
{
    let error = match kind {
        MissingKind::Empty => PkceError::EmptyVerifier {
            state: state.to_owned(),
        },
        MissingKind::Unexpected(shape) => {
            tracing::error!(
                state,
                ?shape,
                "the cached PKCE verifier has an unrecognized format; treating it as a cache miss"
            );
            PkceError::UnexpectedShape {
                state: state.to_owned(),
                shape,
            }
        }
        MissingKind::Absent => {
            let cause = missing_cause(caches);
            tracing::error!(
                state,
                cause,
                "PKCE is enabled but no verifier was found in cache"
            );
            PkceError::MissingVerifier {
                state: state.to_owned(),
                cause,
            }
        }
    };
    if !matches!(kind, MissingKind::Absent) {
        caches.delete_verifier(cache_key).await;
    }
    Err(error)
}

async fn permissive_miss<Redis, User>(
    caches: &PkceCaches<'_, Redis, User>,
    cache_key: &str,
    state: &str,
    kind: MissingKind,
) where
    Redis: VerifierCache,
    User: VerifierCache,
{
    if !matches!(kind, MissingKind::Absent) {
        caches.delete_verifier(cache_key).await;
    }
    tracing::warn!(
        state,
        "PKCE is enabled but no usable verifier was found in cache; continuing without a code_verifier, set PKCE_STRICT_CACHE_MISS=true to fail fast instead"
    );
}

fn missing_cause<Redis, User>(caches: &PkceCaches<'_, Redis, User>) -> &'static str
where
    Redis: VerifierCache,
    User: VerifierCache,
{
    match caches.redis.map_or_else(|| caches.user.kind(), Redis::kind) {
        PkceCacheKind::RedisUsage => {
            "The authorization and callback were likely handled by different instances, so the verifier was stored on one pod but not found on another."
        }
        PkceCacheKind::UserCache => {
            "The verifier may have expired, been lost on a restart, or the PKCE authorization step was never completed. Configure Redis so all proxy instances share the PKCE verifier."
        }
    }
}
