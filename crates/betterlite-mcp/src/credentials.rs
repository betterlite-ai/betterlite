use base64::{Engine as _, engine::general_purpose::STANDARD};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthValue {
    Text(String),
    Headers(BTreeMap<String, String>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
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
    pub timeout_seconds: f64,
    pub extra_headers: BTreeMap<String, String>,
    pub resolved_auth: Option<Box<dyn ResolvedAuth>>,
    pub aws_auth: Option<Box<dyn ResolvedAuth>>,
    pub auth_value: Option<AuthValue>,
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
        extra_headers: Option<BTreeMap<String, String>>,
        timeout_seconds: Option<f64>,
        resolved_auth: Option<Box<dyn ResolvedAuth>>,
        aws_auth: Option<Box<dyn ResolvedAuth>>,
    ) -> Self {
        let mut client = Self {
            server_url,
            auth_type,
            auth_header_name,
            timeout_seconds: timeout_seconds.unwrap_or(60.0_f64),
            extra_headers: extra_headers.unwrap_or_default(),
            resolved_auth,
            aws_auth,
            auth_value: None,
        };
        if let Some(value) = auth_value {
            client.update_auth_value(value);
        }
        client
    }

    pub fn update_auth_value(&mut self, auth_value: AuthValue) {
        let resolved = match auth_value {
            AuthValue::Headers(headers) => Some(AuthValue::Headers(headers)),
            AuthValue::Text(value) if self.auth_type == McpAuthType::Basic => {
                Some(AuthValue::Text(to_basic_credentials(&value)))
            }
            AuthValue::Text(value) => Some(AuthValue::Text(value)),
        };
        self.auth_value = resolved;
    }

    #[must_use]
    pub fn header_slot(&self, default: &str) -> String {
        if let Some(name) = self.auth_header_name.as_ref() {
            return name.clone();
        }
        if let Some(auth) = self.resolved_auth.as_ref()
            && let Some(name) = auth.header_name()
        {
            return name.to_owned();
        }
        default.to_owned()
    }

    #[must_use]
    pub fn auth_headers(&self) -> BTreeMap<String, String> {
        let mut headers = BTreeMap::new();
        if let Some(auth_value) = self.auth_value.as_ref() {
            match auth_value {
                AuthValue::Headers(values) => headers.extend(values.clone()),
                AuthValue::Text(text) => self.insert_text_auth_header(&mut headers, text),
            }
        }
        let extra_headers = self.auth_header_name.as_ref().map_or_else(
            || self.extra_headers.clone(),
            |slot| {
                if contains_header(&headers, slot) {
                    without_header(&self.extra_headers, slot)
                } else {
                    self.extra_headers.clone()
                }
            },
        );
        headers.extend(extra_headers);
        strip_header_whitespace(headers)
    }

    fn insert_text_auth_header(&self, headers: &mut BTreeMap<String, String>, value: &str) {
        let Some((slot, header_value)) = self.text_auth_header(value) else {
            return;
        };
        headers.insert(slot, header_value);
    }

    fn text_auth_header(&self, value: &str) -> Option<(String, String)> {
        let header = match self.auth_type {
            McpAuthType::BearerToken | McpAuthType::Oauth2 | McpAuthType::Oauth2TokenExchange => (
                self.header_slot("Authorization"),
                format!("Bearer {}", strip_auth_scheme(value, "Bearer")),
            ),
            McpAuthType::Basic => (self.header_slot("Authorization"), format!("Basic {value}")),
            McpAuthType::ApiKey => (self.header_slot("X-API-Key"), value.to_owned()),
            McpAuthType::Authorization => (self.header_slot("Authorization"), value.to_owned()),
            McpAuthType::Token => (
                self.header_slot("Authorization"),
                format!("token {}", strip_auth_scheme(value, "token")),
            ),
            McpAuthType::None => return None,
        };
        Some(header)
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
    let mut parts = auth_value.split_whitespace();
    let Some(first) = parts.next() else {
        return auth_value.to_owned();
    };
    let Some(remainder) = parts.next() else {
        return auth_value.to_owned();
    };
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

fn contains_header(headers: &BTreeMap<String, String>, sought: &str) -> bool {
    if headers.keys().any(|name| name.eq_ignore_ascii_case(sought)) {
        return true;
    }
    false
}

fn without_header(headers: &BTreeMap<String, String>, removed: &str) -> BTreeMap<String, String> {
    let pairs = headers.iter();
    pairs
        .filter(|(name, _)| !name.eq_ignore_ascii_case(removed))
        .map(|(name, value)| (name.clone(), value.clone()))
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
