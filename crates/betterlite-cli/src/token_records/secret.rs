use betterlite_logging::private_json::error::PrivateJsonError;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};

use super::error::TokenRecordError;
use super::record::CliTokenRecord;

// Serde's library adapter creates this private, non-Debug value only during explicit serialization.
#[derive(Serialize)]
#[serde(transparent)]
pub(super) struct ExposedSecret(String);

impl From<&SecretString> for ExposedSecret {
    fn from(secret: &SecretString) -> Self {
        Self(secret.expose_secret().to_owned())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CliTokenSecret {
    pub base_url: String,
    #[serde(
        default,
        serialize_with = "serde_with::As::<Option<serde_with::FromIntoRef<ExposedSecret>>>::serialize"
    )]
    pub key: Option<SecretString>,
    #[serde(
        default,
        serialize_with = "serde_with::As::<serde_with::FromIntoRef<ExposedSecret>>::serialize"
    )]
    pub jwt_token: SecretString,
    #[serde(
        default,
        serialize_with = "serde_with::As::<Option<serde_with::FromIntoRef<ExposedSecret>>>::serialize"
    )]
    pub refresh_token: Option<SecretString>,
    #[serde(default)]
    pub timestamp: f64,
}

pub fn encode_secret(record: &CliTokenRecord) -> Result<SecretString, TokenRecordError> {
    let secret = CliTokenSecret {
        base_url: record.base_url.clone(),
        key: record.key.clone(),
        jwt_token: record.jwt_token.clone(),
        refresh_token: record.refresh_token.clone(),
        timestamp: record.timestamp,
    };
    let blob = serde_json::to_string(&secret).map_err(|source| TokenRecordError::Encode {
        source: PrivateJsonError::Encode(source),
    })?;
    Ok(SecretString::from(blob))
}

#[must_use]
pub fn decode_secret(blob: &str, base_url: &str) -> Option<CliTokenSecret> {
    match serde_json::from_str::<CliTokenSecret>(blob) {
        Ok(secret) if secret.base_url == base_url => Some(secret),
        Ok(_) | Err(_) => None,
    }
}
