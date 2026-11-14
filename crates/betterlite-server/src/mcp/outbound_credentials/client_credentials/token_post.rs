mod tests;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::Method;
use http::header::{ACCEPT, HeaderMap, HeaderName, HeaderValue};
use http::header::{InvalidHeaderName, InvalidHeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde::de::IgnoredAny;
use url::Url;

const FORM_CONTENT_TYPE: &str = "application/x-www-form-urlencoded";
const NON_OBJECT_DETAIL: &str = "token endpoint returned a non-JSON-object body";
const JSON_ACCEPT: &str = "application/json";

#[derive(Debug)]
pub enum TokenEndpointOutcome {
    Success { body: TokenResponseBody },
    Denied { status_code: u16, detail: String },
    Unreachable { detail: String },
}

#[derive(Debug, thiserror::Error)]
enum GrantRequestError {
    #[error("the grant header name is not a valid header name")]
    HeaderName(#[from] InvalidHeaderName),
    #[error("the grant header value is not a valid header value")]
    HeaderValue(#[from] InvalidHeaderValue),
}

#[derive(Debug, Deserialize)]
pub struct TokenResponseBody {
    #[serde(default)]
    pub access_token: Option<StringField>,
    #[serde(default)]
    pub expires_in: Option<ExpiresIn>,
    #[serde(default)]
    pub scope: Option<StringField>,
}

impl TokenResponseBody {
    // LiteLLM reads a non-string access_token as absent; the emptiness check belongs to the caller.
    #[must_use]
    pub fn access_token_text(&self) -> Option<&str> {
        match &self.access_token {
            Some(StringField::Text(text)) => Some(text),
            Some(StringField::Other(_)) | None => None,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum StringField {
    Text(String),
    Other(IgnoredAny),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum ExpiresIn {
    Flag(bool),
    Text(String),
    Number(serde_json::Number),
    Other(IgnoredAny),
}

#[async_trait]
pub trait M2mTokenEndpointPost: Send + Sync {
    async fn post(
        &self,
        url: &str,
        form: &[(String, SecretString)],
        headers: &[(String, SecretString)],
    ) -> TokenEndpointOutcome;
}

pub struct HttpGrantPost {
    pub client: Arc<ProxiedClient>,
    pub deadline: Duration,
}

#[async_trait]
impl M2mTokenEndpointPost for HttpGrantPost {
    async fn post(
        &self,
        url: &str,
        form: &[(String, SecretString)],
        headers: &[(String, SecretString)],
    ) -> TokenEndpointOutcome {
        let Ok(parsed) = Url::parse(url) else {
            tracing::warn!("OAuth2 client_credentials POST failed: the token URL is not a URL");
            return TokenEndpointOutcome::Unreachable {
                detail: "InvalidUrl".to_owned(),
            };
        };
        let origin = parsed.origin().ascii_serialization();
        let request = match grant_request(&parsed, form, headers, self.deadline) {
            Ok(request) => request,
            Err(error) => {
                tracing::warn!(upstream = %origin, error = %error, "OAuth2 grant POST failed");
                return TokenEndpointOutcome::Unreachable {
                    detail: "Build".to_owned(),
                };
            }
        };
        let sent = match self.client.send(request).await {
            Ok(sent) => sent,
            Err(error) => return unreachable(&origin, &error),
        };
        let status = sent.response.status();
        if !status.is_success() {
            tracing::warn!(status = status.as_u16(), upstream = %origin, "OAuth2 client_credentials token request denied");
            return TokenEndpointOutcome::Denied {
                status_code: status.as_u16(),
                detail: format!("token endpoint returned HTTP {}", status.as_u16()),
            };
        }
        let buffered = match sent.buffered(self.client.limits.body_bytes).await {
            Ok(response) => response,
            Err(error) => return unreachable(&origin, &error),
        };
        let outcome = parse_token_response(&buffered.body, status.as_u16());
        if let TokenEndpointOutcome::Success { body } = &outcome
            && body.access_token_text().is_none_or(str::is_empty)
        {
            tracing::warn!(status = status.as_u16(), upstream = %origin, "OAuth2 client_credentials response has no access token");
        }
        outcome
    }
}

fn grant_request(
    url: &Url,
    form: &[(String, SecretString)],
    headers: &[(String, SecretString)],
    deadline: Duration,
) -> Result<OutboundRequest, GrantRequestError> {
    let mut serializer = form_urlencoded::Serializer::new(String::new());
    for (name, value) in form {
        serializer.append_pair(name, value.expose_secret());
    }
    let data = serializer.finish().into_bytes();
    let mut map = HeaderMap::new();
    map.insert(ACCEPT, HeaderValue::from_static(JSON_ACCEPT));
    for (name, value) in headers {
        let header_name = HeaderName::try_from(name.as_str())?;
        let mut header_value = HeaderValue::try_from(value.expose_secret())?;
        header_value.set_sensitive(true);
        map.insert(header_name, header_value);
    }
    Ok(OutboundRequest {
        method: Method::POST,
        url: url.clone(),
        query: Vec::new(),
        headers: map,
        body: RequestBody::Bytes {
            content_type: HeaderValue::from_static(FORM_CONTENT_TYPE),
            data,
        },
        deadline,
    })
}

fn unreachable(origin: &str, error: &TransportError) -> TokenEndpointOutcome {
    let kind = transport_kind(error);
    tracing::warn!(upstream = %origin, kind, "OAuth2 client_credentials POST failed");
    TokenEndpointOutcome::Unreachable {
        detail: kind.to_owned(),
    }
}

// The detail names the failure kind only: a URL, a form or a body never reaches a log or an error.
const fn transport_kind(error: &TransportError) -> &'static str {
    match error {
        TransportError::Rejected(_) => "Rejected",
        TransportError::Build(_) => "Build",
        TransportError::Connect(_) => "Connect",
        TransportError::ConnectTimeout(_) => "ConnectTimeout",
        TransportError::Deadline(_) => "Deadline",
        TransportError::Request(_) => "Request",
        TransportError::Body(_) => "Body",
        TransportError::HeadersTooLarge { .. } => "HeadersTooLarge",
        TransportError::BodyTooLarge { .. } => "BodyTooLarge",
        TransportError::InvalidHeader { .. } => "InvalidHeader",
        TransportError::MultipartContentType { .. } => "MultipartContentType",
        TransportError::MultipartBoundaryHeader { .. } => "MultipartBoundaryHeader",
        TransportError::MultipartBoundary { .. } => "MultipartBoundary",
        TransportError::Mock(_) => "Mock",
        TransportError::Closed => "Closed",
        TransportError::DrainTimeout { .. } => "DrainTimeout",
        TransportError::TooManyRedirects { .. } => "TooManyRedirects",
        TransportError::RedirectTarget { .. } => "RedirectTarget",
        TransportError::RedirectUrl { .. } => "RedirectUrl",
        TransportError::RedirectUserinfo => "RedirectUserinfo",
        TransportError::RedirectScheme { .. } => "RedirectScheme",
        TransportError::InsecureRedirect => "InsecureRedirect",
        TransportError::RedirectBody => "RedirectBody",
    }
}

#[must_use]
pub fn parse_token_response(bytes: &[u8], status_code: u16) -> TokenEndpointOutcome {
    let object = serde_json::from_slice::<BTreeMap<String, IgnoredAny>>(bytes).is_ok();
    match serde_json::from_slice::<TokenResponseBody>(bytes) {
        Ok(body) if object => TokenEndpointOutcome::Success { body },
        _ => TokenEndpointOutcome::Denied {
            status_code,
            detail: NON_OBJECT_DETAIL.to_owned(),
        },
    }
}
