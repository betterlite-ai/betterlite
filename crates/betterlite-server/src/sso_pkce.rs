use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use betterlite_config::sso_pkce::PkceSettings;
use secrecy::SecretString;
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use thiserror::Error;
use uuid::Uuid;

pub mod token_exchange;
pub mod verifier_cache;

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
        "PKCE verifier for state '{state}' has an unrecognized format ({shape:?}); the cached entry may be corrupt"
    )]
    UnexpectedShape {
        state: String,
        shape: PkceCacheShape,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PkceErrorResponse {
    pub status: u16,
    pub error_type: &'static str,
    pub param: Option<&'static str>,
    pub code: &'static str,
}

impl PkceError {
    // LiteLLM raises every cache-miss path as a 401 auth_error with param PKCE_CACHE_MISS
    #[must_use]
    pub const fn response(&self) -> PkceErrorResponse {
        match self {
            Self::Entropy(_) => PkceErrorResponse {
                status: 500,
                error_type: "internal_server_error",
                param: None,
                code: "500",
            },
            Self::MissingVerifier { .. }
            | Self::EmptyVerifier { .. }
            | Self::UnexpectedShape { .. } => PkceErrorResponse {
                status: 401,
                error_type: "auth_error",
                param: Some(PKCE_CACHE_MISS_PARAM),
                code: "401",
            },
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
