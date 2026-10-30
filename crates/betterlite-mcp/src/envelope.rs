mod crypto;
pub mod models;
mod tests;

use chrono::{DateTime, TimeDelta, Utc};
use secrecy::SecretString;

use self::crypto::{decrypt_text, encode_jwt, encrypt_grant, open_claims};
use self::models::{WireGrant, WireRefresh};

pub type EnvelopeIdentity = models::EnvelopeIdentity;
pub type EnvelopeKeys = models::EnvelopeKeys;
pub type EnvelopeMintError = models::EnvelopeMintError;
pub type EnvelopeOpenError = models::EnvelopeOpenError;
pub type OpenedEnvelope = models::OpenedEnvelope;
pub type OpenedRefreshEnvelope = models::OpenedRefreshEnvelope;
pub type RefreshCredential = models::RefreshCredential;
pub type SealedEnvelope = models::SealedEnvelope;
pub type UpstreamTokenGrant = models::UpstreamTokenGrant;

pub const ENVELOPE_PREFIX: &str = "llm_env_";
pub const REFRESH_ENVELOPE_PREFIX: &str = "llm_refresh_";
pub const ENVELOPE_ISSUER: &str = "litellm-mcp-bridge";
pub const MAX_ENVELOPE_TTL_SECONDS: i64 = 3_600;
pub const MAX_REFRESH_TTL_SECONDS: i64 = 1_209_600;
pub const MAX_ENVELOPE_BYTES: usize = 12_288;

macro_rules! open_request {
    (access, $candidate:expr, $keys:expr, $now:expr) => {
        OpenRequest {
            candidate: $candidate,
            keys: $keys,
            now: $now,
            prefix: ENVELOPE_PREFIX,
            kind: "access",
            field: "mcp_envelope_grant",
            parse: parse_wire_grant,
        }
    };
    (refresh, $candidate:expr, $keys:expr, $now:expr) => {
        OpenRequest {
            candidate: $candidate,
            keys: $keys,
            now: $now,
            prefix: REFRESH_ENVELOPE_PREFIX,
            kind: "refresh",
            field: "mcp_envelope_refresh",
            parse: parse_wire_refresh,
        }
    };
}

macro_rules! mint_request {
    (access, $identity:expr, $grant:expr, $keys:expr, $now:expr, $lifetime:expr) => {{
        let plaintext = serde_json::to_string(&WireGrant::from($grant))
            .map_err(|_error| EnvelopeMintError::EncryptionFailed)?;
        mint_sealed(MintRequest {
            identity: $identity,
            plaintext: &plaintext,
            keys: $keys,
            now: $now,
            lifetime: $lifetime,
            prefix: ENVELOPE_PREFIX,
            kind: "access",
        })
    }};
    (refresh, $identity:expr, $refresh:expr, $keys:expr, $now:expr, $lifetime:expr) => {{
        let plaintext = serde_json::to_string(&WireRefresh::from($refresh))
            .map_err(|_error| EnvelopeMintError::EncryptionFailed)?;
        mint_sealed(MintRequest {
            identity: $identity,
            plaintext: &plaintext,
            keys: $keys,
            now: $now,
            lifetime: $lifetime,
            prefix: REFRESH_ENVELOPE_PREFIX,
            kind: "refresh",
        })
    }};
}

#[must_use]
pub fn is_envelope(candidate: &str) -> bool {
    if candidate.starts_with(ENVELOPE_PREFIX) {
        return true;
    }
    false
}

#[must_use]
pub fn is_refresh_envelope(candidate: &str) -> bool {
    if candidate.starts_with(REFRESH_ENVELOPE_PREFIX) {
        return true;
    }
    false
}

macro_rules! define_mint {
    ($name:ident, $value:ident: $type:ty, $lifetime:expr, $route:ident) => {
        pub fn $name(
            identity: EnvelopeIdentity,
            $value: $type,
            keys: &EnvelopeKeys,
            now: DateTime<Utc>,
        ) -> Result<SealedEnvelope, EnvelopeMintError> {
            let lifetime = $lifetime;
            mint_request!($route, identity, $value, keys, now, lifetime)
        }
    };
}

macro_rules! define_open {
    ($name:ident, $output:ident, $field:ident, $convert:path, $route:ident) => {
        pub fn $name(
            candidate: &str,
            keys: &EnvelopeKeys,
            now: DateTime<Utc>,
        ) -> Result<$output, EnvelopeOpenError> {
            match open_grant(&open_request!($route, candidate, keys, now)) {
                Ok((identity, wire)) => Ok($output {
                    identity,
                    $field: $convert(wire),
                }),
                Err(error) => Err(error),
            }
        }
    };
}

define_mint!(
    mint_envelope,
    grant: UpstreamTokenGrant,
    grant.expires_in.unwrap_or(MAX_ENVELOPE_TTL_SECONDS),
    access
);
define_mint!(
    mint_refresh_envelope,
    refresh: RefreshCredential,
    refresh
        .expires_in
        .map_or(MAX_REFRESH_TTL_SECONDS, |value| value.min(MAX_REFRESH_TTL_SECONDS)),
    refresh
);
define_open!(
    open_envelope,
    OpenedEnvelope,
    grant,
    UpstreamTokenGrant::from,
    access
);
define_open!(
    open_refresh_envelope,
    OpenedRefreshEnvelope,
    refresh,
    RefreshCredential::from,
    refresh
);

struct OpenRequest<'request, T> {
    candidate: &'request str,
    keys: &'request EnvelopeKeys,
    now: DateTime<Utc>,
    prefix: &'request str,
    kind: &'request str,
    field: &'request str,
    parse: fn(&str) -> Result<T, EnvelopeOpenError>,
}

fn open_grant<T>(request: &OpenRequest<'_, T>) -> Result<(EnvelopeIdentity, T), EnvelopeOpenError> {
    let claims = open_claims(
        request.candidate,
        request.prefix,
        request.kind,
        request.keys,
        request.now,
    )?;
    let plaintext = decrypt_text(&claims.grant, &request.keys.encryption_key, request.field)?;
    let value = (request.parse)(&plaintext)?;
    let identity = EnvelopeIdentity {
        server_id: claims.server_id,
        subject_type: claims.subject_type,
        subject: claims.subject,
    };
    Ok((identity, value))
}

fn parse_wire_grant(text: &str) -> Result<WireGrant, EnvelopeOpenError> {
    match serde_json::from_str(text) {
        Ok(wire) => Ok(wire),
        Err(_error) => Err(EnvelopeOpenError::MalformedPayload),
    }
}

fn parse_wire_refresh(text: &str) -> Result<WireRefresh, EnvelopeOpenError> {
    match serde_json::from_str(text) {
        Ok(wire) => Ok(wire),
        Err(_error) => Err(EnvelopeOpenError::MalformedPayload),
    }
}

struct MintRequest<'request> {
    identity: EnvelopeIdentity,
    plaintext: &'request str,
    keys: &'request EnvelopeKeys,
    now: DateTime<Utc>,
    lifetime: i64,
    prefix: &'request str,
    kind: &'request str,
}

fn mint_sealed(request: MintRequest<'_>) -> Result<SealedEnvelope, EnvelopeMintError> {
    let expires_at = expiry(request.now, request.lifetime)?;
    let grant = encrypt_grant(request.plaintext, &request.keys.encryption_key)?;
    let claims = models::Claims {
        iss: ENVELOPE_ISSUER.to_owned(),
        iat: request.now.timestamp(),
        exp: expires_at.timestamp(),
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
        expires_at,
    })
}

fn expiry(now: DateTime<Utc>, lifetime: i64) -> Result<DateTime<Utc>, EnvelopeMintError> {
    let Some(delta) = TimeDelta::try_seconds(lifetime) else {
        return Err(EnvelopeMintError::EnvelopeUnrepresentable {
            expires_in: lifetime,
        });
    };
    now.checked_add_signed(delta)
        .ok_or(EnvelopeMintError::EnvelopeUnrepresentable {
            expires_in: lifetime,
        })
}
