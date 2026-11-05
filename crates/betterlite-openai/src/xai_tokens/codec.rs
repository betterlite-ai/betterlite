use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use secrecy::SecretString;
use serde::Deserialize;
use serde::de::{DeserializeOwned, IgnoredAny};
use serde_json::Number;

pub const DOCUMENT_LIMIT: usize = 0x0010_0000;

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("document exceeds 1 MiB")]
    Size,
    #[error("unknown document tag")]
    Tag,
    #[error("invalid document")]
    Json(#[source] serde_json::Error),
}

// A number or numeric text keeps its value; any other type counts as unconvertible.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Numeric {
    Number(Number),
    Text(String),
    Other(IgnoredAny),
}

impl Numeric {
    #[must_use]
    pub fn decimal(&self) -> Option<Decimal> {
        let text = match self {
            Self::Number(number) => number.to_string(),
            Self::Text(text) => text.trim().to_owned(),
            Self::Other(_) => return None,
        };
        if text.contains(['e', 'E']) {
            Decimal::from_scientific(&text).ok()
        } else {
            Decimal::from_str_exact(&text).ok()
        }
    }

    // int(value) for a truthy value; None when the value is falsy or cannot be converted.
    #[must_use]
    pub fn seconds(&self) -> Option<i64> {
        match self {
            Self::Number(_) => {
                let value = self.decimal().filter(|value| !value.is_zero())?;
                value.trunc().to_i64()
            }
            Self::Text(text) => text.trim().parse().ok(),
            Self::Other(_) => None,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct AuthFile {
    pub access_token: Option<SecretString>,
    pub refresh_token: Option<SecretString>,
    pub token_endpoint: Option<String>,
    pub expires_at: Option<Numeric>,
}

#[derive(Debug, Deserialize)]
pub struct DiscoveryDocument {
    pub authorization_endpoint: Option<String>,
    pub token_endpoint: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TokenPayload {
    pub access_token: Option<SecretString>,
    pub refresh_token: Option<SecretString>,
    pub id_token: Option<SecretString>,
    pub token_type: Option<String>,
    pub expires_in: Option<Numeric>,
}

#[derive(Debug)]
pub enum Document {
    Auth(AuthFile),
    Discovery(DiscoveryDocument),
    Token(TokenPayload),
}

pub fn decode_xai_oauth<T>(bytes: &[u8]) -> Result<T, DecodeError>
where
    T: DeserializeOwned,
{
    if bytes.len() > DOCUMENT_LIMIT {
        return Err(DecodeError::Size);
    }
    serde_json::from_slice(bytes).map_err(DecodeError::Json)
}

// The first byte picks the document: 0 auth file, 1 discovery, 2 token response.
pub fn decode_tagged(bytes: &[u8]) -> Result<Document, DecodeError> {
    match bytes.split_first() {
        Some((0, rest)) => decode_xai_oauth(rest).map(Document::Auth),
        Some((1, rest)) => decode_xai_oauth(rest).map(Document::Discovery),
        Some((2, rest)) => decode_xai_oauth(rest).map(Document::Token),
        _ => Err(DecodeError::Tag),
    }
}
