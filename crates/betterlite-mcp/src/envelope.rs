mod crypto;
pub mod models;
mod tests;

use betterlite_secrets::secret_codec::NonceSource;
use chrono::{DateTime, TimeDelta, Utc};
use secrecy::{ExposeSecret as _, SecretString};

use self::crypto::{decrypt_text, encode_jwt, encrypt_grant, open_claims};
use self::models::{WireGrant, WireRefresh};

pub type EnvelopeIdentity = models::EnvelopeIdentity;
pub type EnvelopeKeys = models::EnvelopeKeys;
use self::models::EnvelopeKind;
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

#[must_use]
pub fn is_envelope(candidate: &str) -> bool {
    let prefix = ENVELOPE_PREFIX;
    if candidate.is_empty() {
        return false;
    }
    candidate.starts_with(prefix)
}

#[must_use]
pub fn is_refresh_envelope(candidate: &str) -> bool {
    let prefix = REFRESH_ENVELOPE_PREFIX;
    if candidate.is_empty() {
        return false;
    }
    candidate.starts_with(prefix)
}

pub fn mint_envelope<N>(
    identity: EnvelopeIdentity,
    grant: UpstreamTokenGrant,
    keys: &EnvelopeKeys,
    now: DateTime<Utc>,
    nonces: &N,
) -> Result<SealedEnvelope, EnvelopeMintError>
where
    N: NonceSource,
{
    let access_token = grant.access_token.expose_secret();
    let token_type = grant.token_type.as_str();
    if access_token.is_empty() || token_type.is_empty() {
        return Err(EnvelopeMintError::TokenEmpty);
    }
    models::validate_mint(&models::MintValidation {
        identity: &identity,
        keys,
        expires_in: grant.expires_in,
        required_token: Some(grant.access_token.expose_secret()),
        token_type: Some(&grant.token_type),
        optional_token: grant
            .refresh_token
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
    })?;
    let lifetime = grant.expires_in.unwrap_or(MAX_ENVELOPE_TTL_SECONDS);
    let access_prefix = ENVELOPE_PREFIX;
    let access_kind = EnvelopeKind::Access;
    let access_wire = WireGrant::from(grant);
    let plaintext = serde_json::to_string(&access_wire)
        .map_err(|source| EnvelopeMintError::Serialize { source })?;
    let request = MintRequest {
        identity,
        plaintext: &plaintext,
        keys,
        now,
        lifetime,
        prefix: access_prefix,
        kind: access_kind,
        nonces,
    };
    mint_sealed(request)
}

pub fn mint_refresh_envelope<N>(
    identity: EnvelopeIdentity,
    refresh: RefreshCredential,
    keys: &EnvelopeKeys,
    now: DateTime<Utc>,
    nonces: &N,
) -> Result<SealedEnvelope, EnvelopeMintError>
where
    N: NonceSource,
{
    models::validate_refresh_mint(&identity, &refresh, keys)?;
    let refresh_cap = MAX_REFRESH_TTL_SECONDS;
    let lifetime = refresh
        .expires_in
        .map_or(refresh_cap, |value| value.min(refresh_cap));
    let plaintext = serde_json::to_string(&WireRefresh::from(refresh))
        .map_err(|source| EnvelopeMintError::Serialize { source })?;
    mint_sealed(MintRequest {
        identity,
        plaintext: &plaintext,
        keys,
        now,
        lifetime,
        prefix: REFRESH_ENVELOPE_PREFIX,
        kind: EnvelopeKind::Refresh,
        nonces,
    })
}

pub fn open_envelope(
    candidate: &str,
    keys: &EnvelopeKeys,
    now: DateTime<Utc>,
) -> Result<OpenedEnvelope, EnvelopeOpenError> {
    if candidate.len() > MAX_ENVELOPE_BYTES {
        return Err(EnvelopeOpenError::MalformedPayload);
    }
    let access_prefix = ENVELOPE_PREFIX;
    if !candidate.starts_with(access_prefix) {
        return Err(EnvelopeOpenError::NotAnEnvelope);
    }
    let access_kind = EnvelopeKind::Access;
    let access_now = now;
    let request = OpenRequest {
        candidate,
        keys,
        now: access_now,
        prefix: access_prefix,
        kind: access_kind,
    };
    let (identity, plaintext) = open_grant(&request)?;
    let wire: WireGrant = serde_json::from_str(plaintext.expose_secret())
        .map_err(|_error| EnvelopeOpenError::MalformedPayload)?;
    if !wire.is_valid() {
        return Err(EnvelopeOpenError::MalformedPayload);
    }
    Ok(OpenedEnvelope {
        identity,
        grant: UpstreamTokenGrant::from(wire),
    })
}

pub fn open_refresh_envelope(
    candidate: &str,
    keys: &EnvelopeKeys,
    now: DateTime<Utc>,
) -> Result<OpenedRefreshEnvelope, EnvelopeOpenError> {
    if candidate.len() < REFRESH_ENVELOPE_PREFIX.len() {
        return Err(EnvelopeOpenError::NotAnEnvelope);
    }
    let refresh_kind = EnvelopeKind::Refresh;
    let (identity, plaintext) = open_grant(&OpenRequest {
        candidate,
        keys,
        now,
        prefix: REFRESH_ENVELOPE_PREFIX,
        kind: refresh_kind,
    })?;
    let wire = parse_refresh_grant(&plaintext)?;
    Ok(OpenedRefreshEnvelope {
        identity,
        refresh: RefreshCredential::from(wire),
    })
}

struct OpenRequest<'request> {
    candidate: &'request str,
    keys: &'request EnvelopeKeys,
    now: DateTime<Utc>,
    prefix: &'request str,
    kind: EnvelopeKind,
}

fn open_grant(
    request: &OpenRequest<'_>,
) -> Result<(EnvelopeIdentity, SecretString), EnvelopeOpenError> {
    let claims = open_claims(
        request.candidate,
        request.prefix,
        request.kind,
        request.keys,
        request.now,
    )?;
    let plaintext = decrypt_text(&claims.grant, &request.keys.encryption_key)?;
    let identity = EnvelopeIdentity {
        server_id: claims.server_id,
        subject_type: claims.subject_type,
        subject: claims.subject,
    };
    Ok((identity, plaintext))
}

fn parse_refresh_grant(plaintext: &SecretString) -> Result<WireRefresh, EnvelopeOpenError> {
    let wire: WireRefresh = serde_json::from_str(plaintext.expose_secret())
        .map_err(|_error| EnvelopeOpenError::MalformedPayload)?;
    if !wire.is_valid() {
        return Err(EnvelopeOpenError::MalformedPayload);
    }
    Ok(wire)
}

struct MintRequest<'request, N> {
    identity: EnvelopeIdentity,
    plaintext: &'request str,
    keys: &'request EnvelopeKeys,
    now: DateTime<Utc>,
    lifetime: i64,
    prefix: &'request str,
    kind: EnvelopeKind,
    nonces: &'request N,
}

fn mint_sealed<N>(request: MintRequest<'_, N>) -> Result<SealedEnvelope, EnvelopeMintError>
where
    N: NonceSource,
{
    let expires_at = expiry(request.now, request.lifetime)?;
    let grant = encrypt_grant(
        request.plaintext,
        &request.keys.encryption_key,
        request.nonces,
    )
    .map_err(|source| EnvelopeMintError::Seal { source })?;
    let claims = models::Claims {
        iss: ENVELOPE_ISSUER.to_owned(),
        iat: request.now.timestamp(),
        exp: expires_at.timestamp(),
        kind: request.kind,
        server_id: request.identity.server_id,
        subject_type: request.identity.subject_type,
        subject: request.identity.subject,
        grant,
    };
    let compact = encode_jwt(&claims, &request.keys.signing_key)
        .map_err(|source| EnvelopeMintError::Serialize { source })?;
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
