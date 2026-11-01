mod tests;

use std::collections::BTreeMap;

use std::fmt::Write as _;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenEndpointAuthMethod {
    ClientSecretBasic,
    ClientSecretPost,
}

impl TokenEndpointAuthMethod {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ClientSecretBasic => "client_secret_basic",
            Self::ClientSecretPost => "client_secret_post",
        }
    }
}

#[must_use]
pub fn normalize_token_auth_method(value: &str) -> Option<TokenEndpointAuthMethod> {
    match value {
        "client_secret_basic" => Some(TokenEndpointAuthMethod::ClientSecretBasic),
        "client_secret_post" => Some(TokenEndpointAuthMethod::ClientSecretPost),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenEndpointClientAuth {
    pub headers: BTreeMap<String, String>,
    pub body: BTreeMap<String, String>,
}

#[derive(Debug, Error, PartialEq, Eq)]
#[error("token_endpoint_auth_method=client_secret_basic requires both client_id and client_secret")]
pub struct TokenEndpointAuthConfigError;

// RFC 3986 unreserved set with Python quote_plus's space-to-+ rule, matching litellm byte for byte.
fn quote_plus(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-' | b'~' => {
                encoded.push(char::from(byte));
            }
            b' ' => encoded.push('+'),
            _ => {
                write!(encoded, "%{byte:02X}").unwrap_or(());
            }
        }
    }
    encoded
}

pub fn build_token_client_auth(
    auth_method: Option<TokenEndpointAuthMethod>,
    client_id: Option<&str>,
    client_secret: Option<&str>,
) -> Result<TokenEndpointClientAuth, TokenEndpointAuthConfigError> {
    if matches!(
        auth_method,
        Some(TokenEndpointAuthMethod::ClientSecretBasic)
    ) {
        let (Some(id), Some(secret)) = (
            client_id.filter(|value| !value.is_empty()),
            client_secret.filter(|value| !value.is_empty()),
        ) else {
            return Err(TokenEndpointAuthConfigError);
        };
        let userpass = format!("{}:{}", quote_plus(id), quote_plus(secret));
        let mut headers = BTreeMap::new();
        headers.insert(
            "Authorization".to_owned(),
            format!("Basic {}", STANDARD.encode(userpass.as_bytes())),
        );
        return Ok(TokenEndpointClientAuth {
            headers,
            body: BTreeMap::new(),
        });
    }
    let mut body = BTreeMap::new();
    if let Some(id) = client_id.filter(|value| !value.is_empty()) {
        body.insert("client_id".to_owned(), id.to_owned());
    }
    if let Some(secret) = client_secret.filter(|value| !value.is_empty()) {
        body.insert("client_secret".to_owned(), secret.to_owned());
    }
    Ok(TokenEndpointClientAuth {
        headers: BTreeMap::new(),
        body,
    })
}
