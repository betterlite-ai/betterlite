mod tests;

use std::collections::BTreeMap;
use std::fmt;
use std::fmt::Write as _;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use secrecy::{ExposeSecret as _, SecretString};
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

#[derive(Clone)]
pub struct TokenEndpointClientAuth {
    pub headers: BTreeMap<String, SecretString>,
    pub body: BTreeMap<String, SecretString>,
}

impl fmt::Debug for TokenEndpointClientAuth {
    // Redacts every value: a header carries the Basic-encoded secret, and the body can carry client_secret.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokenEndpointClientAuth")
            .field("header_names", &self.headers.keys().collect::<Vec<_>>())
            .field("body_keys", &self.body.keys().collect::<Vec<_>>())
            .finish()
    }
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
    client_secret: Option<&SecretString>,
) -> Result<TokenEndpointClientAuth, TokenEndpointAuthConfigError> {
    if matches!(
        auth_method,
        Some(TokenEndpointAuthMethod::ClientSecretBasic)
    ) {
        let (Some(id), Some(secret)) = (
            client_id.filter(|value| !value.is_empty()),
            client_secret.filter(|value| !value.expose_secret().is_empty()),
        ) else {
            return Err(TokenEndpointAuthConfigError);
        };
        // The secret is exposed here only to build the Basic-encoded header value that the token request actually sends.
        let userpass = format!("{}:{}", quote_plus(id), quote_plus(secret.expose_secret()));
        let mut headers = BTreeMap::new();
        headers.insert(
            "Authorization".to_owned(),
            SecretString::from(format!("Basic {}", STANDARD.encode(userpass.as_bytes()))),
        );
        return Ok(TokenEndpointClientAuth {
            headers,
            body: BTreeMap::new(),
        });
    }
    let mut body = BTreeMap::new();
    if let Some(id) = client_id.filter(|value| !value.is_empty()) {
        body.insert("client_id".to_owned(), SecretString::from(id.to_owned()));
    }
    if let Some(secret) = client_secret.filter(|value| !value.expose_secret().is_empty()) {
        body.insert("client_secret".to_owned(), secret.clone());
    }
    Ok(TokenEndpointClientAuth {
        headers: BTreeMap::new(),
        body,
    })
}
