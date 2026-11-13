use betterlite_config::sso_pkce::PkceSettings;
use secrecy::SecretString;

use super::verifier_cache::{PkceCacheEntry, PkceCacheKind, PkceCaches, VerifierCache};
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

pub async fn prepare_token_exchange_parameters<Redis, User>(
    state: Option<&str>,
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
    let Some(state) = state.filter(|value| !value.is_empty()) else {
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
    match caches.read_verifier(&cache_key).await {
        Some(PkceCacheEntry::Verifier(verifier)) => {
            tracing::debug!("PKCE code verifier retrieved from cache");
            parameters.verifier = Some(PkceVerifier {
                cache_key,
                code_verifier: verifier,
            });
        }
        Some(PkceCacheEntry::LegacyVerifier(verifier)) => {
            tracing::warn!(
                "Retrieved the PKCE code verifier in the legacy plain-string format; future storage uses the object format"
            );
            parameters.verifier = Some(PkceVerifier {
                cache_key,
                code_verifier: verifier,
            });
        }
        Some(PkceCacheEntry::EmptyVerifier) => {
            tracing::warn!(
                state,
                "the PKCE verifier cache entry has an empty or null code_verifier value; treating it as a cache miss"
            );
            handle_missing_pkce_verifier(settings, caches, &cache_key, state, MissingKind::Empty)
                .await?;
        }
        Some(PkceCacheEntry::UnexpectedShape(shape)) => {
            tracing::debug!(
                state,
                %shape,
                "the PKCE verifier cache entry has an unexpected shape; treating it as a cache miss"
            );
            handle_missing_pkce_verifier(
                settings,
                caches,
                &cache_key,
                state,
                MissingKind::Unexpected(shape),
            )
            .await?;
        }
        None => {
            handle_missing_pkce_verifier(settings, caches, &cache_key, state, MissingKind::Absent)
                .await?;
        }
    }
    Ok(parameters)
}

enum MissingKind {
    Absent,
    Empty,
    Unexpected(PkceCacheShape),
}

async fn handle_missing_pkce_verifier<Redis, User>(
    settings: &PkceSettings,
    caches: &PkceCaches<'_, Redis, User>,
    cache_key: &str,
    state: &str,
    kind: MissingKind,
) -> Result<(), PkceError>
where
    Redis: VerifierCache,
    User: VerifierCache,
{
    if !settings.strict_cache_miss {
        if !matches!(kind, MissingKind::Absent) {
            caches.delete_verifier(cache_key).await;
        }
        tracing::warn!(
            state,
            "PKCE is enabled but no usable verifier was found in cache; continuing without a code_verifier, set PKCE_STRICT_CACHE_MISS=true to fail fast instead"
        );
        return Ok(());
    }
    match kind {
        MissingKind::Empty => {
            caches.delete_verifier(cache_key).await;
            Err(PkceError::EmptyVerifier {
                state: state.to_owned(),
            })
        }
        MissingKind::Unexpected(shape) => {
            tracing::error!(
                state,
                %shape,
                "the cached PKCE verifier has an unrecognized format; treating it as a cache miss"
            );
            caches.delete_verifier(cache_key).await;
            Err(PkceError::UnexpectedShape {
                state: state.to_owned(),
                shape,
            })
        }
        MissingKind::Absent => {
            let cause = missing_cause(caches.kind());
            tracing::error!(
                state,
                cause,
                "PKCE is enabled but no verifier was found in cache"
            );
            Err(PkceError::MissingVerifier {
                state: state.to_owned(),
                cause,
            })
        }
    }
}

fn missing_cause(kind: PkceCacheKind) -> &'static str {
    match kind {
        PkceCacheKind::RedisUsage => {
            "The authorization and callback were likely handled by different instances, so the verifier was stored on one pod but not found on another."
        }
        PkceCacheKind::UserCache => {
            "The verifier may have expired, been lost on a restart, or the PKCE authorization step was never completed. Configure Redis so all proxy instances share the PKCE verifier."
        }
    }
}
