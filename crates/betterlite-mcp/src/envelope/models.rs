use chrono::{DateTime, Utc};
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use thiserror::Error;

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
pub(crate) struct Claims {
    pub(crate) iss: String,
    pub(crate) iat: i64,
    pub(crate) exp: i64,
    pub(crate) kind: String,
    pub(crate) server_id: String,
    pub(crate) subject_type: String,
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

#[derive(Debug, Serialize, Deserialize)]
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireRefresh {
    pub(crate) refresh_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) scope: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) expires_in: Option<i64>,
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
