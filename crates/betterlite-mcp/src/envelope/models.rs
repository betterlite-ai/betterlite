mod wire;

use betterlite_secrets::error::SecretError;
use chrono::{DateTime, Utc};
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub(crate) use self::wire::{WireGrant, WireRefresh};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvelopeIdentity {
    pub server_id: String,
    pub subject_type: SubjectType,
    pub subject: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvelopeKind {
    Access,
    Refresh,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubjectType {
    KeyHash,
    UserId,
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

#[derive(Debug, Error)]
pub enum EnvelopeMintError {
    #[error("envelope lifetime cannot be represented: {expires_in} seconds")]
    EnvelopeUnrepresentable { expires_in: i64 },
    #[error("envelope is {size_bytes} bytes, over the {max_bytes} byte limit")]
    EnvelopeTooLarge { size_bytes: usize, max_bytes: usize },
    #[error("signing key must be at least 32 bytes")]
    SigningKeyTooShort,
    #[error("encryption key must not be empty")]
    EncryptionKeyEmpty,
    #[error("envelope identity fields must not be empty")]
    IdentityFieldEmpty,
    #[error("token fields must not be empty")]
    TokenEmpty,
    #[error("expires_in must be positive")]
    ExpiresInInvalid,
    #[error("envelope sealing failed")]
    Seal {
        #[source]
        source: SecretError,
    },
    #[error("envelope serialization failed")]
    Serialize {
        #[source]
        source: serde_json::Error,
    },
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

pub(crate) struct MintValidation<'request> {
    pub(crate) identity: &'request EnvelopeIdentity,
    pub(crate) expires_in: Option<i64>,
    pub(crate) required_token: &'request str,
    pub(crate) token_type: Option<&'request str>,
    pub(crate) optional_token: Option<&'request str>,
}

pub(crate) fn validate_mint(request: &MintValidation<'_>) -> Result<(), EnvelopeMintError> {
    if request.identity.server_id.is_empty() || request.identity.subject.is_empty() {
        return Err(EnvelopeMintError::IdentityFieldEmpty);
    }
    if request.required_token.is_empty()
        || request.token_type.is_some_and(str::is_empty)
        || request.optional_token.is_some_and(str::is_empty)
    {
        return Err(EnvelopeMintError::TokenEmpty);
    }
    if request.expires_in.is_some_and(|value| value <= 0) {
        return Err(EnvelopeMintError::ExpiresInInvalid);
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Claims {
    pub(crate) iss: String,
    pub(crate) iat: i64,
    pub(crate) exp: i64,
    pub(crate) kind: EnvelopeKind,
    pub(crate) server_id: String,
    pub(crate) subject_type: SubjectType,
    pub(crate) subject: String,
    pub(crate) grant: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct Header<'header> {
    pub(crate) alg: &'header str,
    pub(crate) typ: &'header str,
}

#[derive(Debug, Deserialize)]
pub(crate) struct HeaderOwned {
    pub(crate) alg: String,
    pub(crate) typ: String,
}

pub(crate) trait MintPayload: Sized {
    fn expires_in(&self) -> Option<i64>;
    fn required_token(&self) -> &str;
    fn token_type(&self) -> Option<&str>;
    fn optional_token(&self) -> Option<&str>;
    fn lifetime(&self) -> i64;
    fn into_plaintext(self) -> Result<String, EnvelopeMintError>;
}

impl MintPayload for UpstreamTokenGrant {
    fn expires_in(&self) -> Option<i64> {
        self.expires_in
    }

    fn required_token(&self) -> &str {
        self.access_token.expose_secret()
    }

    fn token_type(&self) -> Option<&str> {
        Some(&self.token_type)
    }

    fn optional_token(&self) -> Option<&str> {
        self.refresh_token.as_ref().map(SecretString::expose_secret)
    }

    fn lifetime(&self) -> i64 {
        self.expires_in.unwrap_or(super::MAX_ENVELOPE_TTL_SECONDS)
    }

    fn into_plaintext(self) -> Result<String, EnvelopeMintError> {
        serde_json::to_string(&WireGrant::from(self))
            .map_err(|source| EnvelopeMintError::Serialize { source })
    }
}

impl MintPayload for RefreshCredential {
    fn expires_in(&self) -> Option<i64> {
        self.expires_in
    }

    fn required_token(&self) -> &str {
        self.refresh_token.expose_secret()
    }

    fn token_type(&self) -> Option<&str> {
        None
    }

    fn optional_token(&self) -> Option<&str> {
        None
    }

    fn lifetime(&self) -> i64 {
        self.expires_in
            .map_or(super::MAX_REFRESH_TTL_SECONDS, |value| {
                value.min(super::MAX_REFRESH_TTL_SECONDS)
            })
    }

    fn into_plaintext(self) -> Result<String, EnvelopeMintError> {
        serde_json::to_string(&WireRefresh::from(self))
            .map_err(|source| EnvelopeMintError::Serialize { source })
    }
}
