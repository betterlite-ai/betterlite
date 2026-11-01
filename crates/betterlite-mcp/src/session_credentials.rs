use crate::errors::{SessionKeyError, SessionOpenError as OpenError};
use crate::session_tokens::{SessionPrefix as _, open_session_token};
use crate::types::{
    AsymmetricSessionKeys, SessionKeys, SessionPrincipal, SessionRotatedPublicKey,
    SessionSigningKeys, SessionTokenKind,
};
use betterlite_config::sections::mcp_bearer_credentials::{
    ResolvedSigning, Settings as SigningConfig,
};
use chrono::{DateTime, Utc};
use scrypt::errors::{InvalidOutputLen, InvalidParams};
use scrypt::{Params, scrypt};
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

pub const SESSION_SIGNING_KEY_DOMAIN: &[u8] = b"litellm-mcp-gateway:session-signing:";
const SCRYPT_LOG_N: u8 = 15;
const SCRYPT_R: u32 = 8;
const SCRYPT_P: u32 = 1;
const DERIVED_KEY_BYTES: usize = 32;

#[derive(Debug, Error)]
pub enum SessionKeyDerivationError {
    #[error("scrypt parameters are invalid")]
    ScryptParams(#[from] InvalidParams),
    #[error("scrypt output length is invalid")]
    ScryptOutput(#[from] InvalidOutputLen),
    #[error("derived session key is invalid")]
    Keys(#[from] SessionKeyError),
}

// a domain label distinct from every bridge envelope salt keeps the two credential families isolated.
pub fn session_keys_from_master_key(
    master_key: &SecretString,
) -> Result<SessionKeys, SessionKeyDerivationError> {
    let signing = scrypt_hex(master_key.expose_secret(), SESSION_SIGNING_KEY_DOMAIN)?;
    Ok(SessionKeys::new(SecretString::from(signing))?)
}

// exposed so the oracle can compare it against LiteLLM's hashlib.scrypt(...).hex().
pub fn scrypt_hex(master_key: &str, domain: &[u8]) -> Result<String, SessionKeyDerivationError> {
    let params = Params::new(SCRYPT_LOG_N, SCRYPT_R, SCRYPT_P)?;
    let mut output = [0_u8; DERIVED_KEY_BYTES];
    scrypt(master_key.as_bytes(), domain, &params, &mut output)?;
    Ok(hex::encode(output))
}

// returned rather than raised so issuance and admission fail closed instead of falling back to HS256.
#[derive(Debug, Clone)]
pub struct SessionSigningConfigError {
    pub detail: String,
}

// never substitutes HS256 for a broken RS256 configuration; a resolution defect is passed through, not swallowed.
pub fn resolve_session_signing_keys(
    master_key: &SecretString,
    settings: &SigningConfig,
) -> Result<SessionSigningKeys, SessionSigningConfigError> {
    match settings {
        SigningConfig::Absent => session_keys_from_master_key(master_key)
            .map(SessionSigningKeys::Symmetric)
            .map_err(|source| SessionSigningConfigError {
                detail: source.to_string(),
            }),
        SigningConfig::Invalid(detail) => Err(SessionSigningConfigError {
            detail: detail.clone(),
        }),
        SigningConfig::Resolved(resolved) => build_asymmetric_keys(resolved)
            .map(SessionSigningKeys::Asymmetric)
            .map_err(|source| SessionSigningConfigError {
                detail: source.to_string(),
            }),
    }
}

fn build_asymmetric_keys(
    resolved: &ResolvedSigning,
) -> Result<AsymmetricSessionKeys, SessionKeyError> {
    let mut previous_public_keys = Vec::with_capacity(resolved.previous_public_keys.len());
    for key in &resolved.previous_public_keys {
        previous_public_keys.push(SessionRotatedPublicKey::new(
            key.kid.clone(),
            key.public_key_pem.expose_secret(),
        )?);
    }
    AsymmetricSessionKeys::new(
        &resolved.private_key_pem,
        resolved.kid.clone(),
        previous_public_keys,
    )
}

fn strip_bearer(value: &str) -> &str {
    let Some((scheme, rest)) = value.split_once(char::is_whitespace) else {
        return value;
    };
    if scheme.eq_ignore_ascii_case("bearer") {
        rest.trim_start()
    } else {
        value
    }
}

// a plain OAuth2 bearer falls through to normal admission instead of matching here.
#[must_use]
pub fn is_session_bearer_shaped(authorization_value: &str) -> bool {
    let candidate = strip_bearer(authorization_value);
    candidate.is_session_token() || candidate.is_session_refresh_token()
}

#[derive(Debug, Clone)]
pub enum SessionBearerResult {
    NotSessionBearer,
    SessionBearerAdmitted {
        principal: SessionPrincipal,
        jti: String,
    },
    SessionBearerInvalid {
        // true only for an expired token, never for a tampered or foreign one.
        expired: bool,
    },
}

// a refresh token is SessionBearerInvalid here since it authenticates nothing but a token-endpoint call.
#[must_use]
pub fn resolve_session_bearer(
    authorization_value: &str,
    keys: &SessionSigningKeys,
    now: DateTime<Utc>,
) -> SessionBearerResult {
    let candidate = strip_bearer(authorization_value);
    if candidate.is_session_refresh_token() {
        return SessionBearerResult::SessionBearerInvalid { expired: false };
    }
    if !candidate.is_session_token() {
        return SessionBearerResult::NotSessionBearer;
    }
    match open_session_token(candidate, SessionTokenKind::Session, keys, now) {
        Ok(opened) => SessionBearerResult::SessionBearerAdmitted {
            principal: opened.principal,
            jti: opened.jti,
        },
        Err(OpenError::Expired) => SessionBearerResult::SessionBearerInvalid { expired: true },
        Err(_) => SessionBearerResult::SessionBearerInvalid { expired: false },
    }
}

#[derive(Debug, Clone)]
pub enum SessionRefreshResult {
    SessionRefreshOpened {
        principal: SessionPrincipal,
        jti: String,
    },
    SessionRefreshInvalid,
}

// binding to expected_client_id stops a refresh token issued to one client from renewing a session for another.
#[must_use]
pub fn open_session_refresh_bearer(
    refresh_value: &str,
    keys: &SessionSigningKeys,
    now: DateTime<Utc>,
    expected_client_id: &str,
) -> SessionRefreshResult {
    let candidate = strip_bearer(refresh_value);
    if !candidate.is_session_refresh_token() {
        return SessionRefreshResult::SessionRefreshInvalid;
    }
    let Ok(opened) = open_session_token(candidate, SessionTokenKind::Refresh, keys, now) else {
        return SessionRefreshResult::SessionRefreshInvalid;
    };
    if opened.principal.client_id != expected_client_id {
        return SessionRefreshResult::SessionRefreshInvalid;
    }
    SessionRefreshResult::SessionRefreshOpened {
        principal: opened.principal,
        jti: opened.jti,
    }
}

mod tests;
