use secrecy::SecretString;
use serde::Deserialize;
use serde::de::IgnoredAny;

use super::OidcExchangeError;

#[derive(Deserialize)]
struct RawToken {
    access_token: Option<TextField>,
    expires_in: Option<NumberField>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum TextField {
    Text(String),
    Other(IgnoredAny),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum NumberField {
    Number(i64),
    Other(IgnoredAny),
}

#[derive(Debug)]
pub struct AzureAdToken {
    pub access_token: Option<SecretString>,
    pub expires_in: Option<i64>,
}

pub fn decode_azure_oidc_token(bytes: &[u8]) -> Result<AzureAdToken, OidcExchangeError> {
    let parsed: RawToken =
        serde_json::from_slice(bytes).map_err(|source| OidcExchangeError::TokenJson { source })?;
    let access_token = match parsed.access_token {
        Some(TextField::Text(token)) => Some(SecretString::from(token)),
        Some(TextField::Other(_)) => {
            return Err(OidcExchangeError::TokenFieldType {
                field: "access_token",
            });
        }
        None => None,
    };
    let expires_in = match parsed.expires_in {
        Some(NumberField::Number(value)) => Some(value),
        Some(NumberField::Other(_)) => {
            return Err(OidcExchangeError::TokenFieldType {
                field: "expires_in",
            });
        }
        None => None,
    };
    Ok(AzureAdToken {
        access_token,
        expires_in,
    })
}
