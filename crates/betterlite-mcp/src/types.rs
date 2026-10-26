use std::collections::BTreeSet;
use std::iter;

use aws_lc_rs::encoding::AsDer as _;
use aws_lc_rs::rsa::{KeyPair, PublicEncryptingKey, PublicKey};
use aws_lc_rs::signature::KeyPair as _;
use chrono::{DateTime, Utc};
use pem::{EncodeConfig, LineEnding, Pem};
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};

use crate::errors::{SessionKeyError, SessionOpenError};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionAudience {
    #[serde(rename = "proxy_api")]
    ProxyApi,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionTokenKind {
    #[serde(rename = "session")]
    Session,
    #[serde(rename = "session_refresh")]
    Refresh,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionPrincipal {
    pub user_id: String,
    pub client_id: String,
    pub resource_server_id: Option<String>,
    pub audience: Option<SessionAudience>,
    pub team_id: Option<String>,
}

#[derive(Clone, Debug)]
pub struct SessionKeys {
    pub(crate) signing_key: SecretString,
}

impl SessionKeys {
    pub fn new(signing_key: SecretString) -> Result<Self, SessionKeyError> {
        if signing_key.expose_secret().chars().count() < 32 {
            return Err(SessionKeyError::ShortSigningKey);
        }
        Ok(Self { signing_key })
    }
}

#[derive(Clone, Debug)]
pub struct SessionRotatedPublicKey {
    pub(crate) kid: String,
    pub(crate) public_key_der: Vec<u8>,
}

impl SessionRotatedPublicKey {
    pub fn new(kid: String, public_key_pem: &str) -> Result<Self, SessionKeyError> {
        if kid.is_empty() {
            return Err(SessionKeyError::EmptyKid);
        }
        let document =
            pem::parse(public_key_pem).map_err(|source| SessionKeyError::Pem { source })?;
        let public = PublicKey::from_der(document.contents())
            .map_err(|source| SessionKeyError::PublicKey { source })?;
        let spki = public
            .as_der()
            .map_err(|source| SessionKeyError::PublicEncoding { source })?;
        PublicEncryptingKey::from_der(spki.as_ref())
            .map_err(|source| SessionKeyError::PublicKey { source })?;
        Ok(Self {
            kid,
            public_key_der: spki.as_ref().to_vec(),
        })
    }
}

#[derive(Debug)]
pub struct AsymmetricSessionKeys {
    pub(crate) signer: KeyPair,
    pub(crate) kid: String,
    pub(crate) previous_public_keys: Vec<SessionRotatedPublicKey>,
}

impl AsymmetricSessionKeys {
    pub fn new(
        private_key_pem: &SecretString,
        kid: String,
        previous_public_keys: Vec<SessionRotatedPublicKey>,
    ) -> Result<Self, SessionKeyError> {
        let document = pem::parse(private_key_pem.expose_secret())
            .map_err(|source| SessionKeyError::Pem { source })?;
        let parsed = if document.tag() == "RSA PRIVATE KEY" {
            KeyPair::from_der(document.contents())
        } else {
            KeyPair::from_pkcs8(document.contents())
        };
        let signer = parsed.map_err(|source| SessionKeyError::PrivateKey { source })?;
        if kid.is_empty() {
            return Err(SessionKeyError::EmptyKid);
        }
        let mut seen = BTreeSet::new();
        let mut duplicated = BTreeSet::new();
        for candidate in iter::once(&kid).chain(previous_public_keys.iter().map(|key| &key.kid)) {
            if !seen.insert(candidate) {
                duplicated.insert(candidate.as_str());
            }
        }
        if !duplicated.is_empty() {
            let kids = duplicated.into_iter().collect::<Vec<_>>().join(", ");
            return Err(SessionKeyError::DuplicateKid { kids });
        }
        Ok(Self {
            signer,
            kid,
            previous_public_keys,
        })
    }

    pub fn public_key_pem(&self) -> Result<String, SessionKeyError> {
        let public = self
            .signer
            .public_key()
            .as_der()
            .map_err(|source| SessionKeyError::PublicEncoding { source })?;
        Ok(pem::encode_config(
            &Pem::new("PUBLIC KEY", public.as_ref()),
            EncodeConfig::new().set_line_ending(LineEnding::LF),
        ))
    }

    pub(crate) fn verification_key(&self, kid: Option<&str>) -> Result<&[u8], SessionOpenError> {
        if kid == Some(self.kid.as_str()) {
            return Ok(self.signer.public_key().as_ref());
        }
        self.previous_public_keys
            .iter()
            .find(|previous| kid == Some(previous.kid.as_str()))
            .map(|previous| previous.public_key_der.as_slice())
            .ok_or(SessionOpenError::BadSignature)
    }
}

#[derive(Debug)]
pub enum SessionSigningKeys {
    Symmetric(SessionKeys),
    Asymmetric(AsymmetricSessionKeys),
}

#[derive(Clone, Debug)]
pub struct MintedSessionToken {
    pub token: SecretString,
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OpenedSessionToken {
    pub principal: SessionPrincipal,
    pub jti: String,
    pub kind: SessionTokenKind,
    pub iat: i64,
    pub exp: i64,
}
