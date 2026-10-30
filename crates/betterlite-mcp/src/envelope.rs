mod tests;

use aws_lc_rs::hmac;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use betterlite_secrets::secret_codec::diagnostics::{Level, SecretDiagnostics, SecretEvent};
use betterlite_secrets::secret_codec::{
    Algorithm, CipherSettings, DecryptPolicy, FailureLog, FailureValue, OsNonces, StoredCipher,
    StoredValue,
};
use chrono::{DateTime, TimeDelta, Utc};
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const ENVELOPE_PREFIX: &str = "llm_env_";
pub const REFRESH_ENVELOPE_PREFIX: &str = "llm_refresh_";
pub const ENVELOPE_ISSUER: &str = "litellm-mcp-bridge";
pub const MAX_ENVELOPE_TTL_SECONDS: i64 = 3_600;
pub const MAX_REFRESH_TTL_SECONDS: i64 = 1_209_600;
pub const MAX_ENVELOPE_BYTES: usize = 12_288;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvelopeIdentity {
    pub server_id: String,
    pub subject_type: String,
    pub subject: String,
}

#[derive(Debug, Clone)]
pub struct UpstreamTokenGrant {
    pub access_token: SecretString,
    pub token_type: String,
    pub refresh_token: Option<SecretString>,
    pub scope: Option<String>,
    pub expires_in: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct RefreshCredential {
    pub refresh_token: SecretString,
    pub scope: Option<String>,
    pub expires_in: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct EnvelopeKeys {
    pub signing_key: SecretString,
    pub encryption_key: SecretString,
}

#[derive(Debug, Clone)]
pub struct SealedEnvelope {
    pub token: SecretString,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct OpenedEnvelope {
    pub identity: EnvelopeIdentity,
    pub grant: UpstreamTokenGrant,
}

#[derive(Debug, Clone)]
pub struct OpenedRefreshEnvelope {
    pub identity: EnvelopeIdentity,
    pub refresh: RefreshCredential,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum EnvelopeMintError {
    #[error("envelope lifetime cannot be represented: {expires_in} seconds")]
    EnvelopeUnrepresentable { expires_in: i64 },
    #[error("envelope is {size_bytes} bytes, over the {max_bytes} byte limit")]
    EnvelopeTooLarge { size_bytes: usize, max_bytes: usize },
    #[error("envelope encryption failed")]
    EncryptionFailed,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum EnvelopeOpenError {
    #[error("value does not have the expected envelope prefix")]
    NotAnEnvelope,
    #[error("envelope signature is invalid")]
    BadSignature,
    #[error("envelope has expired")]
    Expired,
    #[error("envelope payload is malformed")]
    MalformedPayload,
    #[error("envelope grant could not be decrypted")]
    DecryptFailed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Claims {
    iss: String,
    iat: i64,
    exp: i64,
    kind: String,
    server_id: String,
    subject_type: String,
    subject: String,
    grant: String,
}

#[derive(Debug, Serialize)]
struct Header<'header> {
    alg: &'header str,
    typ: &'header str,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireGrant {
    access_token: String,
    token_type: String,
    refresh_token: Option<String>,
    scope: Option<String>,
    expires_in: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireRefresh {
    refresh_token: String,
    scope: Option<String>,
    expires_in: Option<i64>,
}

#[must_use]
pub fn is_envelope(candidate: &str) -> bool {
    let has_prefix = candidate.starts_with(ENVELOPE_PREFIX);
    has_prefix
}

#[must_use]
pub fn is_refresh_envelope(candidate: &str) -> bool {
    let has_prefix = candidate.starts_with(REFRESH_ENVELOPE_PREFIX);
    has_prefix
}

pub fn mint_envelope(
    identity: EnvelopeIdentity,
    grant: UpstreamTokenGrant,
    keys: &EnvelopeKeys,
    now: DateTime<Utc>,
) -> Result<SealedEnvelope, EnvelopeMintError> {
    let lifetime = grant.expires_in.unwrap_or(MAX_ENVELOPE_TTL_SECONDS);
    let expires_at = expires_at(now, lifetime)?;
    let plaintext = serde_json::to_string(&WireGrant {
        access_token: grant.access_token.expose_secret().to_owned(),
        token_type: grant.token_type,
        refresh_token: grant
            .refresh_token
            .map(|value| value.expose_secret().to_owned()),
        scope: grant.scope,
        expires_in: grant.expires_in,
    })
    .map_err(|_error| EnvelopeMintError::EncryptionFailed)?;
    seal_envelope(SealRequest {
        prefix: ENVELOPE_PREFIX,
        kind: "access",
        identity,
        plaintext: &plaintext,
        keys,
        now,
        expires_at,
    })
}

pub fn open_envelope(
    candidate: &str,
    keys: &EnvelopeKeys,
    now: DateTime<Utc>,
) -> Result<OpenedEnvelope, EnvelopeOpenError> {
    let claims = open_claims(candidate, ENVELOPE_PREFIX, "access", keys, now)?;
    let grant = decrypt_grant(&claims.grant, &keys.encryption_key)?;
    Ok(OpenedEnvelope {
        identity: identity_from_claims(&claims),
        grant,
    })
}

pub fn mint_refresh_envelope(
    identity: EnvelopeIdentity,
    refresh: RefreshCredential,
    keys: &EnvelopeKeys,
    now: DateTime<Utc>,
) -> Result<SealedEnvelope, EnvelopeMintError> {
    let lifetime = refresh.expires_in.map_or(MAX_REFRESH_TTL_SECONDS, |value| {
        value.min(MAX_REFRESH_TTL_SECONDS)
    });
    let expires_at = expires_at(now, lifetime)?;
    let plaintext = serde_json::to_string(&WireRefresh {
        refresh_token: refresh.refresh_token.expose_secret().to_owned(),
        scope: refresh.scope,
        expires_in: refresh.expires_in,
    })
    .map_err(|_error| EnvelopeMintError::EncryptionFailed)?;
    seal_envelope(SealRequest {
        prefix: REFRESH_ENVELOPE_PREFIX,
        kind: "refresh",
        identity,
        plaintext: &plaintext,
        keys,
        now,
        expires_at,
    })
}

pub fn open_refresh_envelope(
    candidate: &str,
    keys: &EnvelopeKeys,
    now: DateTime<Utc>,
) -> Result<OpenedRefreshEnvelope, EnvelopeOpenError> {
    let claims = open_claims(candidate, REFRESH_ENVELOPE_PREFIX, "refresh", keys, now)?;
    let refresh = decrypt_refresh(&claims.grant, &keys.encryption_key)?;
    Ok(OpenedRefreshEnvelope {
        identity: identity_from_claims(&claims),
        refresh,
    })
}

fn expires_at(now: DateTime<Utc>, lifetime: i64) -> Result<DateTime<Utc>, EnvelopeMintError> {
    let result = TimeDelta::try_seconds(lifetime)
        .and_then(|delta| now.checked_add_signed(delta))
        .ok_or(EnvelopeMintError::EnvelopeUnrepresentable {
            expires_in: lifetime,
        });
    result
}

struct SealRequest<'request> {
    prefix: &'request str,
    kind: &'request str,
    identity: EnvelopeIdentity,
    plaintext: &'request str,
    keys: &'request EnvelopeKeys,
    now: DateTime<Utc>,
    expires_at: DateTime<Utc>,
}

fn seal_envelope(request: SealRequest<'_>) -> Result<SealedEnvelope, EnvelopeMintError> {
    let grant = encrypt_grant(request.plaintext, &request.keys.encryption_key)?;
    let claims = Claims {
        iss: ENVELOPE_ISSUER.to_owned(),
        iat: request.now.timestamp(),
        exp: request.expires_at.timestamp(),
        kind: request.kind.to_owned(),
        server_id: request.identity.server_id,
        subject_type: request.identity.subject_type,
        subject: request.identity.subject,
        grant,
    };
    let compact = encode_jwt(&claims, &request.keys.signing_key)
        .map_err(|_error| EnvelopeMintError::EncryptionFailed)?;
    let token = format!("{}{compact}", request.prefix);
    let size_bytes = token.len();
    if size_bytes > MAX_ENVELOPE_BYTES {
        return Err(EnvelopeMintError::EnvelopeTooLarge {
            size_bytes,
            max_bytes: MAX_ENVELOPE_BYTES,
        });
    }
    Ok(SealedEnvelope {
        token: SecretString::from(token),
        expires_at: request.expires_at,
    })
}

fn encode_jwt(claims: &Claims, key: &SecretString) -> Result<String, serde_json::Error> {
    let header = serde_json::to_vec(&Header {
        alg: "HS256",
        typ: "JWT",
    })?;
    let payload = serde_json::to_vec(claims)?;
    let header_segment = URL_SAFE_NO_PAD.encode(header);
    let payload_segment = URL_SAFE_NO_PAD.encode(payload);
    let signing_input = format!("{header_segment}.{payload_segment}");
    let tag = hmac::sign(
        &hmac::Key::new(hmac::HMAC_SHA256, key.expose_secret().as_bytes()),
        signing_input.as_bytes(),
    );
    Ok(format!(
        "{signing_input}.{}",
        URL_SAFE_NO_PAD.encode(tag.as_ref())
    ))
}

fn open_claims(
    candidate: &str,
    prefix: &str,
    expected_kind: &str,
    keys: &EnvelopeKeys,
    now: DateTime<Utc>,
) -> Result<Claims, EnvelopeOpenError> {
    let compact = candidate
        .strip_prefix(prefix)
        .ok_or(EnvelopeOpenError::NotAnEnvelope)?;
    if candidate.len() > MAX_ENVELOPE_BYTES {
        return Err(EnvelopeOpenError::MalformedPayload);
    }
    if candidate.len() > MAX_ENVELOPE_BYTES {
        return Err(EnvelopeOpenError::MalformedPayload);
    }
    let claims = verify_jwt(compact, &keys.signing_key)?;
    if claims.kind != expected_kind
        || claims.iss != ENVELOPE_ISSUER
        || claims.server_id.is_empty()
        || claims.subject_type.is_empty()
        || claims.subject.is_empty()
        || claims.grant.is_empty()
    {
        return Err(EnvelopeOpenError::MalformedPayload);
    }
    if now.timestamp() >= claims.exp {
        return Err(EnvelopeOpenError::Expired);
    }
    Ok(claims)
}

fn verify_jwt(compact: &str, key: &SecretString) -> Result<Claims, EnvelopeOpenError> {
    let (signing_input, signature_segment) = compact
        .rsplit_once('.')
        .ok_or(EnvelopeOpenError::MalformedPayload)?;
    let (header_segment, payload_segment) = signing_input
        .split_once('.')
        .ok_or(EnvelopeOpenError::MalformedPayload)?;
    let header_bytes = URL_SAFE_NO_PAD
        .decode(header_segment)
        .map_err(|_error| EnvelopeOpenError::MalformedPayload)?;
    let header: HeaderOwned = serde_json::from_slice(&header_bytes)
        .map_err(|_error| EnvelopeOpenError::MalformedPayload)?;
    if header.alg != "HS256" || header.typ != "JWT" {
        return Err(EnvelopeOpenError::MalformedPayload);
    }
    let signature = URL_SAFE_NO_PAD
        .decode(signature_segment)
        .map_err(|_error| EnvelopeOpenError::MalformedPayload)?;
    let hmac_key = hmac::Key::new(hmac::HMAC_SHA256, key.expose_secret().as_bytes());
    if hmac::verify(&hmac_key, signing_input.as_bytes(), &signature).is_err() {
        return Err(EnvelopeOpenError::BadSignature);
    }
    let payload = URL_SAFE_NO_PAD
        .decode(payload_segment)
        .map_err(|_error| EnvelopeOpenError::MalformedPayload)?;
    serde_json::from_slice(&payload).map_err(|_error| EnvelopeOpenError::MalformedPayload)
}

#[derive(Debug, Deserialize)]
struct HeaderOwned {
    alg: String,
    typ: String,
}

fn identity_from_claims(claims: &Claims) -> EnvelopeIdentity {
    let identity = EnvelopeIdentity {
        server_id: claims.server_id.clone(),
        subject_type: claims.subject_type.clone(),
        subject: claims.subject.clone(),
    };
    identity
}

fn encrypt_grant(plaintext: &str, key: &SecretString) -> Result<String, EnvelopeMintError> {
    let cipher = cipher(key);
    let encrypted = cipher
        .encrypt_value(StoredValue::<()>::Text(SecretString::from(plaintext)), None)
        .map_err(|_error| EnvelopeMintError::EncryptionFailed)?;
    let StoredValue::Text(sealed_text) = encrypted else {
        return Err(EnvelopeMintError::EncryptionFailed);
    };
    Ok(URL_SAFE_NO_PAD.encode(sealed_text.expose_secret()))
}

fn decrypt_grant(
    encoded: &str,
    key: &SecretString,
) -> Result<UpstreamTokenGrant, EnvelopeOpenError> {
    let decoded = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_error| EnvelopeOpenError::DecryptFailed)?;
    let text = String::from_utf8(decoded).map_err(|_error| EnvelopeOpenError::DecryptFailed)?;
    let cipher = cipher(key);
    let decrypted = cipher.decrypt_value(
        StoredValue::<()>::Text(SecretString::from(text)),
        "mcp_envelope_grant",
        DecryptPolicy {
            log: FailureLog::Debug,
            value: FailureValue::Nothing,
        },
    );
    let Some(StoredValue::Text(opened_text)) = decrypted else {
        return Err(EnvelopeOpenError::DecryptFailed);
    };
    let wire: WireGrant = serde_json::from_str(opened_text.expose_secret())
        .map_err(|_error| EnvelopeOpenError::MalformedPayload)?;
    Ok(UpstreamTokenGrant {
        access_token: SecretString::from(wire.access_token),
        token_type: wire.token_type,
        refresh_token: wire.refresh_token.map(SecretString::from),
        scope: wire.scope,
        expires_in: wire.expires_in,
    })
}

fn decrypt_refresh(
    encoded: &str,
    key: &SecretString,
) -> Result<RefreshCredential, EnvelopeOpenError> {
    let decoded = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_error| EnvelopeOpenError::DecryptFailed)?;
    let text = String::from_utf8(decoded).map_err(|_error| EnvelopeOpenError::DecryptFailed)?;
    let cipher = cipher(key);
    let decrypted = cipher.decrypt_value(
        StoredValue::<()>::Text(SecretString::from(text)),
        "mcp_envelope_refresh",
        DecryptPolicy {
            log: FailureLog::Debug,
            value: FailureValue::Nothing,
        },
    );
    let Some(StoredValue::Text(opened_text)) = decrypted else {
        return Err(EnvelopeOpenError::DecryptFailed);
    };
    let wire: WireRefresh = serde_json::from_str(opened_text.expose_secret())
        .map_err(|_error| EnvelopeOpenError::MalformedPayload)?;
    Ok(RefreshCredential {
        refresh_token: SecretString::from(wire.refresh_token),
        scope: wire.scope,
        expires_in: wire.expires_in,
    })
}

struct NoopDiagnostics;

impl SecretDiagnostics for NoopDiagnostics {
    fn record(&self, _level: Level, _event: &SecretEvent<'_>) {}
}

fn cipher(key: &SecretString) -> StoredCipher<OsNonces, NoopDiagnostics> {
    let settings = CipherSettings {
        signing_key: Some(key.clone()),
        algorithm: Algorithm::XSalsa20Poly1305,
    };
    let cipher = StoredCipher {
        settings,
        nonces: OsNonces,
        diagnostics: NoopDiagnostics,
    };
    cipher
}
