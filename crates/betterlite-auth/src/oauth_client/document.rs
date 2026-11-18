use std::collections::BTreeMap;

use betterlite_http::proxy::limits::TransportLimits;
use betterlite_models::controls::json_value::JsonValue;
use secrecy::SecretString;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Deserializer;
use thiserror::Error;

pub const MAX_OAUTH_DOCUMENT_BYTES: usize = TransportLimits::DEFAULT.body_bytes;

#[derive(Debug, Error)]
pub enum OAuthDocumentError {
    #[error("the OAuth provider document is {bytes} bytes; the maximum is {limit}")]
    TooLarge { bytes: usize, limit: usize },
    #[error("the OAuth provider document kind tag {tag} is unknown")]
    UnknownKind { tag: u8 },
    #[error("the OAuth provider document has bytes after its JSON value: {0}")]
    Trailing(#[source] serde_json::Error),
    #[error("the OAuth provider document does not match its shape: {0}")]
    Document(#[from] serde_path_to_error::Error<serde_json::Error>),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct OAuthDiscoveryDocument {
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub userinfo_endpoint: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OAuthTokenResponse {
    pub access_token: SecretString,
    #[serde(default)]
    pub refresh_token: Option<SecretString>,
    #[serde(default)]
    pub id_token: Option<SecretString>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(transparent)]
pub struct OAuthUserInfo {
    pub fields: BTreeMap<String, JsonValue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OAuthDocumentKind {
    Discovery,
    Token,
    Userinfo,
}

#[derive(Debug)]
pub enum OAuthProviderDocument {
    Discovery(OAuthDiscoveryDocument),
    Token(OAuthTokenResponse),
    Userinfo(OAuthUserInfo),
}

impl TryFrom<u8> for OAuthDocumentKind {
    type Error = OAuthDocumentError;

    fn try_from(tag: u8) -> Result<Self, Self::Error> {
        match tag {
            0 => Ok(Self::Discovery),
            1 => Ok(Self::Token),
            2 => Ok(Self::Userinfo),
            other => Err(OAuthDocumentError::UnknownKind { tag: other }),
        }
    }
}

impl OAuthDocumentKind {
    pub fn decode(self, bytes: &[u8]) -> Result<OAuthProviderDocument, OAuthDocumentError> {
        match self {
            Self::Discovery => {
                decode_oauth_provider_document(bytes).map(OAuthProviderDocument::Discovery)
            }
            Self::Token => decode_oauth_provider_document(bytes).map(OAuthProviderDocument::Token),
            Self::Userinfo => {
                decode_oauth_provider_document(bytes).map(OAuthProviderDocument::Userinfo)
            }
        }
    }
}

pub fn decode_oauth_provider_document<T>(bytes: &[u8]) -> Result<T, OAuthDocumentError>
where
    T: DeserializeOwned,
{
    check_size(bytes)?;
    let mut decoder = Deserializer::from_slice(bytes);
    let document: T = serde_path_to_error::deserialize(&mut decoder)?;
    decoder.end().map_err(OAuthDocumentError::Trailing)?;
    Ok(document)
}

const fn check_size(bytes: &[u8]) -> Result<(), OAuthDocumentError> {
    if bytes.len() > MAX_OAUTH_DOCUMENT_BYTES {
        return Err(OAuthDocumentError::TooLarge {
            bytes: bytes.len(),
            limit: MAX_OAUTH_DOCUMENT_BYTES,
        });
    }
    Ok(())
}
