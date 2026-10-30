use betterlite_secrets::error::SecretError;
use chrono::{DateTime, Utc};
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use thiserror::Error;

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
    pub(crate) keys: &'request EnvelopeKeys,
    pub(crate) expires_in: Option<i64>,
    pub(crate) required_token: Option<&'request str>,
    pub(crate) token_type: Option<&'request str>,
    pub(crate) optional_token: Option<&'request str>,
}

pub(crate) fn validate_refresh_mint(
    identity: &EnvelopeIdentity,
    refresh: &RefreshCredential,
    keys: &EnvelopeKeys,
) -> Result<(), EnvelopeMintError> {
    let validation = validate_mint(&MintValidation {
        identity,
        keys,
        expires_in: refresh.expires_in,
        required_token: Some(refresh.refresh_token.expose_secret()),
        token_type: None,
        optional_token: None,
    });
    validation?;
    Ok(())
}

pub(crate) fn validate_mint(request: &MintValidation<'_>) -> Result<(), EnvelopeMintError> {
    if request.keys.signing_key.expose_secret().len() < 32 {
        return Err(EnvelopeMintError::SigningKeyTooShort);
    }
    if request.keys.encryption_key.expose_secret().is_empty() {
        return Err(EnvelopeMintError::EncryptionKeyEmpty);
    }
    if request.identity.server_id.is_empty() || request.identity.subject.is_empty() {
        return Err(EnvelopeMintError::IdentityFieldEmpty);
    }
    if request.required_token.is_some_and(str::is_empty)
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

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireGrant {
    pub(crate) access_token: String,
    pub(crate) token_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) refresh_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) scope: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) expires_in: Option<i64>,
}

impl WireGrant {
    pub(crate) fn is_valid(&self) -> bool {
        if self.access_token.is_empty() || self.token_type.is_empty() {
            return false;
        }
        if self.refresh_token.as_ref().is_some_and(String::is_empty) {
            return false;
        }
        self.expires_in.is_none_or(|value| value > 0)
    }
}

impl From<UpstreamTokenGrant> for WireGrant {
    fn from(grant: UpstreamTokenGrant) -> Self {
        Self {
            access_token: grant.access_token.expose_secret().to_owned(),
            token_type: grant.token_type,
            refresh_token: grant
                .refresh_token
                .map(|value| value.expose_secret().to_owned()),
            scope: grant.scope,
            expires_in: grant.expires_in,
        }
    }
}

impl From<WireGrant> for UpstreamTokenGrant {
    fn from(wire: WireGrant) -> Self {
        Self {
            access_token: SecretString::from(wire.access_token),
            token_type: wire.token_type,
            refresh_token: wire.refresh_token.map(SecretString::from),
            scope: wire.scope,
            expires_in: wire.expires_in,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireRefresh {
    pub(crate) refresh_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) scope: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) expires_in: Option<i64>,
}

impl WireRefresh {
    pub(crate) fn is_valid(&self) -> bool {
        if self.refresh_token.is_empty() {
            return false;
        }
        self.expires_in.is_none_or(|value| value > 0)
    }
}

impl From<RefreshCredential> for WireRefresh {
    fn from(refresh: RefreshCredential) -> Self {
        Self {
            refresh_token: refresh.refresh_token.expose_secret().to_owned(),
            scope: refresh.scope,
            expires_in: refresh.expires_in,
        }
    }
}

impl From<WireRefresh> for RefreshCredential {
    fn from(wire: WireRefresh) -> Self {
        Self {
            refresh_token: SecretString::from(wire.refresh_token),
            scope: wire.scope,
            expires_in: wire.expires_in,
        }
    }
}
