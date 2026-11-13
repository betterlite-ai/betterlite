use std::fmt;

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use betterlite_auth::auth_cache::AuthCache;
use betterlite_auth::auth_cache::backend::AuthCacheTier;
use betterlite_config::sso_pkce::PkceSettings;
use betterlite_models::decisions::Json;
use secrecy::SecretString;
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use thiserror::Error;
use uuid::Uuid;

mod tests;

pub const PKCE_VERIFIER_CACHE_PREFIX: &str = "pkce_verifier:";
pub const PKCE_CACHE_MISS_PARAM: &str = "PKCE_CACHE_MISS";
pub const CODE_CHALLENGE_METHOD: &str = "S256";
const VERIFIER_RANDOM_BYTES: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PkceCacheShape {
    Object,
    Boolean,
    Number,
    List,
    Null,
}

impl fmt::Display for PkceCacheShape {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Object => "object",
            Self::Boolean => "bool",
            Self::Number => "number",
            Self::List => "list",
            Self::Null => "null",
        };
        formatter.write_str(name)
    }
}

#[derive(Debug, Error)]
pub enum PkceError {
    #[error("could not draw random bytes for the PKCE verifier")]
    Entropy(#[source] getrandom::Error),
    #[error("PKCE verifier not found in cache for state '{state}'. {cause}")]
    MissingVerifier { state: String, cause: &'static str },
    #[error(
        "PKCE verifier for state '{state}' was found in cache but has an empty or null code_verifier value"
    )]
    EmptyVerifier { state: String },
    #[error(
        "PKCE verifier for state '{state}' has an unrecognized format ({shape}); the cached entry may be corrupt"
    )]
    UnexpectedShape {
        state: String,
        shape: PkceCacheShape,
    },
}

impl PkceError {
    // LiteLLM raises every cache-miss path as a 401 auth_error with param PKCE_CACHE_MISS
    #[must_use]
    pub const fn status(&self) -> u16 {
        match self {
            Self::Entropy(_) => 500,
            Self::MissingVerifier { .. }
            | Self::EmptyVerifier { .. }
            | Self::UnexpectedShape { .. } => 401,
        }
    }

    #[must_use]
    pub const fn error_type(&self) -> &'static str {
        match self {
            Self::Entropy(_) => "internal_server_error",
            Self::MissingVerifier { .. }
            | Self::EmptyVerifier { .. }
            | Self::UnexpectedShape { .. } => "auth_error",
        }
    }

    #[must_use]
    pub const fn param(&self) -> Option<&'static str> {
        match self {
            Self::Entropy(_) => None,
            Self::MissingVerifier { .. }
            | Self::EmptyVerifier { .. }
            | Self::UnexpectedShape { .. } => Some(PKCE_CACHE_MISS_PARAM),
        }
    }

    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Entropy(_) => "500",
            Self::MissingVerifier { .. }
            | Self::EmptyVerifier { .. }
            | Self::UnexpectedShape { .. } => "401",
        }
    }
}

pub trait VerifierEntropy: Send + Sync {
    fn fill_verifier(
        &self,
        bytes: &mut [u8; VERIFIER_RANDOM_BYTES],
    ) -> Result<(), getrandom::Error>;
}

pub struct OsEntropy;

impl VerifierEntropy for OsEntropy {
    fn fill_verifier(
        &self,
        bytes: &mut [u8; VERIFIER_RANDOM_BYTES],
    ) -> Result<(), getrandom::Error> {
        getrandom::fill(bytes)
    }
}

#[derive(Debug)]
pub struct GeneratedPkce {
    pub verifier: SecretString,
    pub challenge: String,
}

pub fn generate_pkce_params<E>(entropy: &E) -> Result<GeneratedPkce, PkceError>
where
    E: VerifierEntropy,
{
    let mut bytes = [0_u8; VERIFIER_RANDOM_BYTES];
    entropy
        .fill_verifier(&mut bytes)
        .map_err(PkceError::Entropy)?;
    let verifier = URL_SAFE_NO_PAD.encode(bytes);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    Ok(GeneratedPkce {
        verifier: SecretString::from(verifier),
        challenge,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PkceChallengeParams {
    pub code_challenge: String,
    pub code_challenge_method: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GenericSsoRedirectParams {
    pub state: String,
    #[serde(flatten)]
    pub pkce: Option<PkceChallengeParams>,
}

#[derive(Debug)]
pub struct GenericSsoRedirect {
    pub params: GenericSsoRedirectParams,
    pub code_verifier: Option<SecretString>,
}

pub fn generic_sso_redirect_params<E>(
    state: Option<&str>,
    settings: &PkceSettings,
    entropy: &E,
) -> Result<GenericSsoRedirect, PkceError>
where
    E: VerifierEntropy,
{
    let state = match state.filter(|value| !value.is_empty()) {
        Some(cli_state) => cli_state.to_owned(),
        None => settings
            .generic_client_state
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().simple().to_string()),
    };
    if !settings.use_pkce {
        return Ok(GenericSsoRedirect {
            params: GenericSsoRedirectParams { state, pkce: None },
            code_verifier: None,
        });
    }
    let generated = generate_pkce_params(entropy)?;
    tracing::debug!("PKCE enabled for the generic SSO authorization request");
    Ok(GenericSsoRedirect {
        params: GenericSsoRedirectParams {
            state,
            pkce: Some(PkceChallengeParams {
                code_challenge: generated.challenge,
                code_challenge_method: CODE_CHALLENGE_METHOD,
            }),
        },
        code_verifier: Some(generated.verifier),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PkceCacheKind {
    RedisUsage,
    UserCache,
}

#[derive(Debug, Clone)]
pub enum PkceCacheEntry {
    Verifier(SecretString),
    LegacyVerifier(SecretString),
    EmptyVerifier,
    UnexpectedShape(PkceCacheShape),
}

// A cache read failure is a miss and a delete failure is a warning, as both LiteLLM caches do
#[async_trait]
pub trait VerifierCache: Send + Sync {
    fn kind(&self) -> PkceCacheKind;
    async fn read(&self, cache_key: &str) -> Option<PkceCacheEntry>;
    async fn delete(&self, cache_key: &str);
}

#[async_trait]
impl<T: AuthCacheTier> VerifierCache for AuthCache<T> {
    fn kind(&self) -> PkceCacheKind {
        PkceCacheKind::UserCache
    }

    async fn read(&self, cache_key: &str) -> Option<PkceCacheEntry> {
        self.async_get(cache_key, false)
            .await
            .and_then(|value| stored_entry(&value))
    }

    async fn delete(&self, cache_key: &str) {
        if let Err(error) = self.tier(cache_key).async_delete(cache_key).await {
            tracing::warn!(
                %error,
                "PKCE: failed to delete the verifier cache key (best-effort cleanup)"
            );
        }
    }
}

// Both cache tiers read a stored null as a miss, so only a present non-null value reaches this match
fn stored_entry(value: &Json) -> Option<PkceCacheEntry> {
    match value {
        Json::Null => None,
        Json::Object(fields) => Some(object_entry(fields.get("code_verifier"))),
        Json::Text(text) => Some(legacy_entry(text)),
        Json::Bool(_) => Some(PkceCacheEntry::UnexpectedShape(PkceCacheShape::Boolean)),
        Json::Number(_) => Some(PkceCacheEntry::UnexpectedShape(PkceCacheShape::Number)),
        Json::List(_) => Some(PkceCacheEntry::UnexpectedShape(PkceCacheShape::List)),
    }
}

fn object_entry(field: Option<&Json>) -> PkceCacheEntry {
    match field {
        Some(Json::Text(text)) if !text.is_empty() => {
            PkceCacheEntry::Verifier(SecretString::from(text.clone()))
        }
        Some(Json::Text(_)) | Some(Json::Null) => PkceCacheEntry::EmptyVerifier,
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) | None => {
            PkceCacheEntry::UnexpectedShape(PkceCacheShape::Object)
        }
    }
}

fn legacy_entry(text: &str) -> PkceCacheEntry {
    if text.is_empty() {
        PkceCacheEntry::EmptyVerifier
    } else {
        PkceCacheEntry::LegacyVerifier(SecretString::from(text.to_owned()))
    }
}

pub struct PkceCaches<'a, Redis, User> {
    pub redis: Option<&'a Redis>,
    pub user: &'a User,
}

impl<Redis, User> PkceCaches<'_, Redis, User>
where
    Redis: VerifierCache,
    User: VerifierCache,
{
    // LiteLLM reads the Redis usage cache when one is configured and the user cache otherwise
    async fn read_verifier(&self, cache_key: &str) -> Option<PkceCacheEntry> {
        match self.redis {
            Some(redis) => redis.read(cache_key).await,
            None => self.user.read(cache_key).await,
        }
    }

    pub async fn delete_verifier(&self, cache_key: &str) {
        match self.redis {
            Some(redis) => redis.delete(cache_key).await,
            None => self.user.delete(cache_key).await,
        }
    }

    fn kind(&self) -> PkceCacheKind {
        match self.redis {
            Some(redis) => redis.kind(),
            None => self.user.kind(),
        }
    }
}

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
