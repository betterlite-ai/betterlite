use chrono::{DateTime, Utc};
use secrecy::SecretString;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionPrincipal {
    pub user_id: String,
    pub client_id: String,
    pub resource_server_id: Option<String>,
    pub audience: Option<String>,
    pub team_id: Option<String>,
}

#[derive(Clone, Debug)]
pub struct SessionKeys {
    pub signing_key: SecretString,
}

#[derive(Clone, Debug)]
pub struct SessionRotatedPublicKey {
    pub kid: String,
    pub public_key_pem: String,
}

#[derive(Clone, Debug)]
pub struct AsymmetricSessionKeys {
    pub private_key_pem: SecretString,
    pub kid: String,
    pub previous_public_keys: Vec<SessionRotatedPublicKey>,
}

#[derive(Clone, Debug)]
pub enum SessionSigningKeys {
    Symmetric(SessionKeys),
    Asymmetric(AsymmetricSessionKeys),
}

#[derive(Clone, Debug)]
pub struct MintedSessionToken {
    pub token: SecretString,
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenedSessionToken {
    pub principal: SessionPrincipal,
    pub jti: String,
    pub kind: SessionTokenKind,
    pub iat: i64,
    pub exp: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionTokenKind {
    Session,
    Refresh,
}

impl SessionTokenKind {
    pub(crate) const fn claim(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Refresh => "session_refresh",
        }
    }
}
