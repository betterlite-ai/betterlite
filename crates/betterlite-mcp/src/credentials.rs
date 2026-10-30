use base64::{Engine as _, engine::general_purpose::STANDARD};
use secrecy::{ExposeSecret as _, SecretString};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum McpAuthType {
    None,
    ApiKey,
    BearerToken,
    Basic,
    Authorization,
    Oauth2,
    Token,
    Oauth2TokenExchange,
}

#[derive(Clone)]
pub enum AuthValue {
    Text(SecretString),
    Headers(BTreeMap<String, SecretString>),
}

#[derive(Clone, PartialEq, Eq)]
pub struct Request {
    pub method: String,
    pub url: String,
    pub headers: BTreeMap<String, String>,
}

#[derive(Debug, thiserror::Error)]
pub enum CredentialError {
    #[error("could not encode discovery fingerprint material: {source}")]
    FingerprintSerialization {
        #[source]
        source: serde_json::Error,
    },
    #[error("resolved authentication failed")]
    AuthenticationFailed,
}

pub trait ResolvedAuth: Send + Sync {
    fn authenticate(&self, request: Request) -> Result<Request, CredentialError>;
    fn close(&self);
    fn header_name(&self) -> Option<&str> {
        None
    }
}

pub struct McpClient {
    pub server_url: String,
    pub auth_type: McpAuthType,
    pub auth_header_name: Option<String>,
    pub timeout: Duration,
    pub extra_headers: BTreeMap<String, String>,
    pub resolved_auth: Option<Box<dyn ResolvedAuth>>,
    pub aws_auth: Option<Box<dyn ResolvedAuth>>,
    pub auth_value: Option<AuthValue>,
    pub credential_slot: Option<String>,
}

impl McpClient {
    #[expect(
        clippy::too_many_arguments,
        reason = "mirrors LiteLLM's public client constructor"
    )]
    #[must_use]
    pub fn new(
        server_url: String,
        auth_type: McpAuthType,
        auth_value: Option<AuthValue>,
        auth_header_name: Option<String>,
        extra_headers: BTreeMap<String, String>,
        timeout: Duration,
        resolved_auth: Option<Box<dyn ResolvedAuth>>,
        aws_auth: Option<Box<dyn ResolvedAuth>>,
    ) -> Self {
        let credential_slot = auth_header_name.clone().or_else(|| {
            let auth = resolved_auth.as_ref()?;
            auth.header_name().map(str::to_owned)
        });
        let mut client = Self {
            server_url,
            auth_type,
            auth_header_name,
            timeout,
            extra_headers,
            resolved_auth,
            aws_auth,
            credential_slot,
            auth_value: None,
        };
        if let Some(value) = auth_value {
            client.update_auth_value(value);
        }
        client
    }

    pub fn update_auth_value(&mut self, auth_value: AuthValue) {
        let resolved = match auth_value {
            AuthValue::Headers(headers) if !headers.is_empty() => Some(AuthValue::Headers(headers)),
            AuthValue::Headers(_) => None,
            AuthValue::Text(value) if value.expose_secret().is_empty() => None,
            AuthValue::Text(value) if self.auth_type == McpAuthType::Basic => {
                Some(AuthValue::Text(SecretString::from(to_basic_credentials(
                    value.expose_secret(),
                ))))
            }
            AuthValue::Text(value) => Some(AuthValue::Text(value)),
        };
        self.auth_value = resolved;
    }

    #[must_use]
    pub fn auth_headers(&self) -> BTreeMap<String, String> {
        let mut headers = BTreeMap::new();
        if let Some(auth_value) = self.auth_value.as_ref() {
            match auth_value {
                AuthValue::Headers(values) => headers.extend(
                    values
                        .iter()
                        .map(|(name, value)| (name.clone(), value.expose_secret().to_owned())),
                ),
                AuthValue::Text(text) => self.insert_text_auth_header(&mut headers, text),
            }
        }
        let extra_headers = self.credential_slot.as_ref().map_or_else(
            || self.extra_headers.clone(),
            |slot| {
                if headers.keys().any(|name| name.eq_ignore_ascii_case(slot)) {
                    self.extra_headers
                        .iter()
                        .filter(|(name, _)| !name.eq_ignore_ascii_case(slot))
                        .map(|(name, value)| (name.clone(), value.clone()))
                        .collect()
                } else {
                    self.extra_headers.clone()
                }
            },
        );
        headers.extend(extra_headers);
        strip_header_whitespace(headers)
    }

    fn insert_text_auth_header(
        &self,
        headers: &mut BTreeMap<String, String>,
        secret: &SecretString,
    ) {
        let exposed = secret.expose_secret();
        let Some((slot, header_value)) = self.text_auth_header(exposed) else {
            return;
        };
        headers.insert(slot, header_value);
    }

    fn text_auth_header(&self, value: &str) -> Option<(String, String)> {
        let (default_name, header_value) = match self.auth_type {
            McpAuthType::BearerToken | McpAuthType::Oauth2 | McpAuthType::Oauth2TokenExchange => (
                "Authorization",
                format!("Bearer {}", strip_auth_scheme(value, "Bearer")),
            ),
            McpAuthType::Basic => ("Authorization", format!("Basic {value}")),
            McpAuthType::ApiKey => ("X-API-Key", value.to_owned()),
            McpAuthType::Authorization => ("Authorization", value.to_owned()),
            McpAuthType::Token => (
                "Authorization",
                format!("token {}", strip_auth_scheme(value, "token")),
            ),
            McpAuthType::None => return None,
        };
        let slot = self
            .credential_slot
            .clone()
            .unwrap_or_else(|| default_name.to_owned());
        Some((slot, header_value))
    }

    pub fn prepare_request_auth(&self) -> Result<Request, CredentialError> {
        let request = Request {
            method: "POST".to_owned(),
            url: if self.server_url.is_empty() {
                "http://localhost/".to_owned()
            } else {
                self.server_url.clone()
            },
            headers: self.auth_headers(),
        };
        let Some(auth) = self.resolved_auth.as_ref() else {
            return Ok(request);
        };
        let result = auth.authenticate(request);
        auth.close();
        result
    }

    pub fn discovery_auth_fingerprint(&self) -> Result<String, CredentialError> {
        let request = self.prepare_request_auth()?;
        hash_discovery_auth(&request)
    }
}

#[must_use]
pub fn to_basic_auth(auth_value: &str) -> String {
    let bytes = auth_value.as_bytes();
    STANDARD.encode(bytes)
}

#[must_use]
pub fn strip_auth_scheme(auth_value: &str, scheme: &str) -> String {
    let trimmed = auth_value.trim_start();
    let Some((first, raw_remainder)) = trimmed.split_once(char::is_whitespace) else {
        return auth_value.to_owned();
    };
    let remainder = raw_remainder.trim_start();
    if remainder.is_empty() {
        return auth_value.to_owned();
    }
    if first.eq_ignore_ascii_case(scheme) {
        remainder.to_owned()
    } else {
        auth_value.to_owned()
    }
}

#[must_use]
pub fn to_basic_credentials(auth_value: &str) -> String {
    let credentials = strip_auth_scheme(auth_value, "Basic");
    if credentials == auth_value {
        return to_basic_auth(auth_value);
    }
    if STANDARD.decode(credentials.as_bytes()).is_ok() {
        credentials
    } else {
        to_basic_auth(&credentials)
    }
}

#[must_use]
pub fn strip_header_whitespace(headers: BTreeMap<String, String>) -> BTreeMap<String, String> {
    let pairs = headers.into_iter();
    pairs
        .map(|(key, value)| (key.trim().to_owned(), value.trim().to_owned()))
        .collect()
}

fn hash_discovery_auth(request: &Request) -> Result<String, CredentialError> {
    let material = (
        request.url.as_str(),
        request
            .headers
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect::<Vec<_>>(),
    );
    let encoded = serde_json::to_vec(&material)
        .map_err(|source| CredentialError::FingerprintSerialization { source })?;
    Ok(hex::encode(Sha256::digest(encoded)))
}
