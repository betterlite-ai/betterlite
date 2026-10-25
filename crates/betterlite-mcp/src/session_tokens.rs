use crate::{
    errors::SessionTokenError,
    types::{
        AsymmetricSessionKeys, MintedSessionToken, OpenedSessionToken, SessionPrincipal,
        SessionSigningKeys, SessionTokenKind,
    },
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Duration, Utc};
use jsonwebtoken::{
    Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, decode_header, encode,
    errors::ErrorKind,
};
use rsa::{
    RsaPrivateKey,
    pkcs8::{DecodePrivateKey as _, EncodePublicKey as _, LineEnding},
};
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
pub const SESSION_TOKEN_PREFIX: &str = "llm_session_";
pub const SESSION_REFRESH_PREFIX: &str = "llm_srefresh_";
pub const SESSION_ISSUER: &str = "litellm-mcp-gateway";
pub const SESSION_TTL_SECONDS: i64 = 3_600;
pub const SESSION_REFRESH_TTL_SECONDS: i64 = 1_209_600;
pub const MAX_SESSION_TOKEN_BYTES: usize = 4_096;
const ACCESS_SPEC: MintSpec<'static> = MintSpec {
    kind: SessionTokenKind::Session,
    ttl_seconds: SESSION_TTL_SECONDS,
    prefix: SESSION_TOKEN_PREFIX,
};
const REFRESH_SPEC: MintSpec<'static> = MintSpec {
    kind: SessionTokenKind::Refresh,
    ttl_seconds: SESSION_REFRESH_TTL_SECONDS,
    prefix: SESSION_REFRESH_PREFIX,
};
macro_rules! mint_route {
    ($name:ident, $spec:ident) => {
        pub fn $name(
            principal: &SessionPrincipal,
            keys: &SessionSigningKeys,
            now: DateTime<Utc>,
        ) -> Result<MintedSessionToken, SessionTokenError> {
            let result = mint(principal, keys, now, &$spec);
            result
        }
    };
}
macro_rules! open_route {
    ($name:ident, $kind:expr, $prefix:expr) => {
        pub fn $name(
            candidate: &str,
            keys: &SessionSigningKeys,
            now: DateTime<Utc>,
        ) -> Result<OpenedSessionToken, SessionTokenError> {
            let result = open(candidate, keys, now, $kind, $prefix);
            result
        }
    };
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Claims {
    iss: String,
    iat: i64,
    exp: i64,
    jti: String,
    kind: String,
    user_id: String,
    client_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    resource_server_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    audience: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    team_id: Option<String>,
}
#[must_use]
pub fn is_session_token(candidate: &str) -> bool {
    if candidate.starts_with(SESSION_TOKEN_PREFIX) {
        return true;
    }
    false
}
#[must_use]
pub fn is_session_refresh_token(candidate: &str) -> bool {
    if candidate.starts_with(SESSION_REFRESH_PREFIX) {
        return true;
    }
    false
}
mint_route!(mint_session_token, ACCESS_SPEC);
mint_route!(mint_session_refresh_token, REFRESH_SPEC);
open_route!(
    open_session_token,
    SessionTokenKind::Session,
    SESSION_TOKEN_PREFIX
);
open_route!(
    open_session_refresh_token,
    SessionTokenKind::Refresh,
    SESSION_REFRESH_PREFIX
);
pub fn session_public_key_pem(keys: &AsymmetricSessionKeys) -> Result<String, SessionTokenError> {
    let private = RsaPrivateKey::from_pkcs8_pem(keys.private_key_pem.expose_secret())
        .map_err(|_source| SessionTokenError::InvalidKey)?;
    private
        .to_public_key()
        .to_public_key_pem(LineEnding::LF)
        .map_err(|_source| SessionTokenError::InvalidKey)
}
#[derive(Clone, Copy)]
struct MintSpec<'prefix> {
    kind: SessionTokenKind,
    ttl_seconds: i64,
    prefix: &'prefix str,
}
fn mint(
    principal: &SessionPrincipal,
    keys: &SessionSigningKeys,
    now: DateTime<Utc>,
    spec: &MintSpec<'_>,
) -> Result<MintedSessionToken, SessionTokenError> {
    if principal.user_id.is_empty() || principal.client_id.is_empty() {
        return Err(SessionTokenError::Malformed);
    }
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random).map_err(|_source| SessionTokenError::InvalidKey)?;
    let claims = Claims {
        iss: SESSION_ISSUER.to_owned(),
        iat: now.timestamp(),
        exp: now
            .checked_add_signed(Duration::seconds(spec.ttl_seconds))
            .ok_or(SessionTokenError::Malformed)?
            .timestamp(),
        jti: URL_SAFE_NO_PAD.encode(random),
        kind: spec.kind.claim().to_owned(),
        user_id: principal.user_id.clone(),
        client_id: principal.client_id.clone(),
        resource_server_id: principal.resource_server_id.clone(),
        audience: principal.audience.clone(),
        team_id: principal.team_id.clone(),
    };
    let compact = sign(&claims, keys)?;
    let token = format!("{}{compact}", spec.prefix);
    let size_bytes = token.len();
    if size_bytes > MAX_SESSION_TOKEN_BYTES {
        return Err(SessionTokenError::TooLarge {
            size_bytes,
            max_bytes: MAX_SESSION_TOKEN_BYTES,
        });
    }
    Ok(MintedSessionToken {
        token: SecretString::from(token),
        expires_at: DateTime::from_timestamp(claims.exp, 0).ok_or(SessionTokenError::Malformed)?,
    })
}

fn sign(claims: &Claims, keys: &SessionSigningKeys) -> Result<String, SessionTokenError> {
    let material = match keys {
        SessionSigningKeys::Symmetric(session_keys) => {
            if session_keys.signing_key.expose_secret().len() < 32 {
                return Err(SessionTokenError::InvalidKey);
            }
            (
                Header::new(Algorithm::HS256),
                EncodingKey::from_secret(session_keys.signing_key.expose_secret().as_bytes()),
            )
        }
        SessionSigningKeys::Asymmetric(session_keys) => {
            let mut header = Header::new(Algorithm::RS256);
            header.kid = Some(session_keys.kid.clone());
            let key =
                EncodingKey::from_rsa_pem(session_keys.private_key_pem.expose_secret().as_bytes())
                    .map_err(|_source| SessionTokenError::InvalidKey)?;
            (header, key)
        }
    };
    encode(&material.0, claims, &material.1).map_err(|_source| SessionTokenError::InvalidKey)
}

#[expect(
    clippy::too_many_lines,
    reason = "the ordered hostile-input checks mirror LiteLLM's verification pipeline"
)]
fn open(
    candidate: &str,
    keys: &SessionSigningKeys,
    now: DateTime<Utc>,
    expected_kind: SessionTokenKind,
    prefix: &str,
) -> Result<OpenedSessionToken, SessionTokenError> {
    if !candidate.starts_with(prefix) {
        return Err(SessionTokenError::NotASessionToken);
    }
    if candidate.len() > MAX_SESSION_TOKEN_BYTES {
        return Err(SessionTokenError::Malformed);
    }
    let compact = candidate
        .strip_prefix(prefix)
        .ok_or(SessionTokenError::NotASessionToken)?;
    let (key, algorithm) = verification_key(compact, keys)?;
    let mut validation = Validation::new(algorithm);
    validation.validate_exp = false;
    validation.validate_nbf = false;
    validation.validate_aud = false;
    validation.set_issuer(&[SESSION_ISSUER]);
    validation.set_required_spec_claims(&["iss", "iat", "exp"]);
    let data = decode::<Claims>(compact, &key, &validation).map_err(|error| {
        if error.kind() == &ErrorKind::InvalidSignature {
            SessionTokenError::BadSignature
        } else {
            SessionTokenError::Malformed
        }
    })?;
    if data.claims.jti.is_empty()
        || data.claims.user_id.is_empty()
        || data.claims.client_id.is_empty()
        || data.claims.kind != expected_kind.claim()
    {
        return Err(SessionTokenError::Malformed);
    }
    if now.timestamp() >= data.claims.exp {
        return Err(SessionTokenError::Expired);
    }
    Ok(OpenedSessionToken {
        principal: SessionPrincipal {
            user_id: data.claims.user_id,
            client_id: data.claims.client_id,
            resource_server_id: data.claims.resource_server_id,
            audience: data.claims.audience,
            team_id: data.claims.team_id,
        },
        jti: data.claims.jti,
        kind: expected_kind,
        iat: data.claims.iat,
        exp: data.claims.exp,
    })
}

fn verification_key(
    compact: &str,
    keys: &SessionSigningKeys,
) -> Result<(DecodingKey, Algorithm), SessionTokenError> {
    match keys {
        SessionSigningKeys::Symmetric(session_keys) => {
            if session_keys.signing_key.expose_secret().len() < 32 {
                return Err(SessionTokenError::InvalidKey);
            }
            Ok((
                DecodingKey::from_secret(session_keys.signing_key.expose_secret().as_bytes()),
                Algorithm::HS256,
            ))
        }
        SessionSigningKeys::Asymmetric(session_keys) => {
            let header = decode_header(compact).map_err(|_source| SessionTokenError::Malformed)?;
            let kid = header.kid.ok_or(SessionTokenError::BadSignature)?;
            if kid == session_keys.kid {
                return Ok((
                    DecodingKey::from_rsa_pem(session_public_key_pem(session_keys)?.as_bytes())
                        .map_err(|_source| SessionTokenError::InvalidKey)?,
                    Algorithm::RS256,
                ));
            }
            let previous = session_keys
                .previous_public_keys
                .iter()
                .find(|key| key.kid == kid)
                .ok_or(SessionTokenError::BadSignature)?;
            Ok((
                DecodingKey::from_rsa_pem(previous.public_key_pem.as_bytes())
                    .map_err(|_source| SessionTokenError::InvalidKey)?,
                Algorithm::RS256,
            ))
        }
    }
}
