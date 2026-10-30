mod crypto;
pub mod models;
mod tests;

use betterlite_secrets::secret_codec::NonceSource;
use chrono::{DateTime, TimeDelta, Utc};
use secrecy::{ExposeSecret as _, SecretString};

use self::crypto::{decrypt_text, encode_jwt, encrypt_grant, open_claims};
use self::models::{MintPayload as _, MintValidation, WireGrant, WireRefresh};

pub type EnvelopeIdentity = models::EnvelopeIdentity;
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

#[derive(Debug, Clone)]
pub struct EnvelopeKeys {
    signing_key: SecretString,
    encryption_key: SecretString,
}

impl EnvelopeKeys {
    pub fn new(
        signing_key: SecretString,
        encryption_key: SecretString,
    ) -> Result<Self, EnvelopeMintError> {
        if signing_key.expose_secret().len() < 32 {
            return Err(EnvelopeMintError::SigningKeyTooShort);
        }
        if encryption_key.expose_secret().is_empty() {
            return Err(EnvelopeMintError::EncryptionKeyEmpty);
        }
        Ok(Self {
            signing_key,
            encryption_key,
        })
    }
}

pub trait EnvelopePrefix {
    fn is_envelope(&self) -> bool;
    fn is_refresh_envelope(&self) -> bool;
}

impl EnvelopePrefix for str {
    fn is_envelope(&self) -> bool {
        self.starts_with(ENVELOPE_PREFIX)
    }

    fn is_refresh_envelope(&self) -> bool {
        self.starts_with(REFRESH_ENVELOPE_PREFIX)
    }
}

macro_rules! mint_envelope_fn {
    ($name:ident, $payload:ty, $prefix:expr, $kind:expr) => {
        pub fn $name<N>(
            identity: EnvelopeIdentity,
            payload: $payload,
            keys: &EnvelopeKeys,
            now: DateTime<Utc>,
            nonces: &N,
        ) -> Result<SealedEnvelope, EnvelopeMintError>
        where
            N: NonceSource,
        {
            models::validate_mint(&MintValidation {
                identity: &identity,
                expires_in: payload.expires_in,
                required_token: payload.required_token(),
                token_type: payload.token_type(),
            })?;
            let lifetime = payload.lifetime();
            let plaintext = payload.into_plaintext()?;
            mint_sealed(MintRequest {
                identity,
                plaintext: &plaintext,
                keys,
                now,
                lifetime,
                prefix: $prefix,
                kind: $kind,
                nonces,
            })
        }
    };
}

mint_envelope_fn!(
    mint_envelope,
    UpstreamTokenGrant,
    ENVELOPE_PREFIX,
    EnvelopeKind::Access
);
mint_envelope_fn!(
    mint_refresh_envelope,
    RefreshCredential,
    REFRESH_ENVELOPE_PREFIX,
    EnvelopeKind::Refresh
);

macro_rules! open_envelope_fn {
    ($name:ident, $opened:ident, $wire:ty, $payload:ty, $field:ident, $prefix:expr, $kind:expr) => {
        pub fn $name(
            candidate: &str,
            keys: &EnvelopeKeys,
            now: DateTime<Utc>,
        ) -> Result<$opened, EnvelopeOpenError> {
            let (identity, plaintext) = open_grant(&OpenRequest {
                candidate,
                keys,
                now,
                prefix: $prefix,
                kind: $kind,
            })?;
            let wire: $wire = serde_json::from_str(plaintext.expose_secret())
                .map_err(|_error| EnvelopeOpenError::MalformedPayload)?;
            if !wire.is_valid() {
                return Err(EnvelopeOpenError::MalformedPayload);
            }
            Ok($opened {
                identity,
                $field: <$payload>::from(wire),
            })
        }
    };
}

open_envelope_fn!(
    open_envelope,
    OpenedEnvelope,
    WireGrant,
    UpstreamTokenGrant,
    grant,
    ENVELOPE_PREFIX,
    EnvelopeKind::Access
);
open_envelope_fn!(
    open_refresh_envelope,
    OpenedRefreshEnvelope,
    WireRefresh,
    RefreshCredential,
    refresh,
    REFRESH_ENVELOPE_PREFIX,
    EnvelopeKind::Refresh
);

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
