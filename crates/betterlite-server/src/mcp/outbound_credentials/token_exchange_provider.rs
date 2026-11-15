mod tests;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method};
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use url::Url;

use super::refresh_store::InProcessRefreshCoordinator;
use super::token_exchanger::{
    DEFAULT_TTL, EXPIRY_BUFFER, ExchangeHttpPost, ExchangeRejection, ExchangeTokenResponse,
    MIN_TTL, OboTokenExchanger,
};
use super::v2_store::CredentialValue;
use super::{InMemoryTokenCacheBackend, OAuthToken, SystemWallClock, TokenCacheEntries, WallClock};

pub const CACHE_MAX_SIZE: usize = 500;
pub const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(30);

// RFC 6749 5.2 codes that mean the gateway's own request is wrong, not the caller's subject token.
const GATEWAY_FAULT_OAUTH_ERRORS: [&str; 5] = [
    "invalid_client",
    "unauthorized_client",
    "unsupported_grant_type",
    "invalid_target",
    "invalid_scope",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenExchangeTunables {
    pub cache_max_size: usize,
    pub default_ttl: Duration,
    pub min_ttl: Duration,
    pub expiry_buffer: Duration,
}

impl Default for TokenExchangeTunables {
    fn default() -> Self {
        Self {
            cache_max_size: CACHE_MAX_SIZE,
            default_ttl: DEFAULT_TTL,
            min_ttl: MIN_TTL,
            expiry_buffer: EXPIRY_BUFFER,
        }
    }
}

// The two fields the edge reads from an RFC 6749 5.2 error body; error_description is never read.
#[derive(Debug, Default, Deserialize)]
pub struct OAuthErrorBody {
    #[serde(default)]
    pub error: CredentialValue,
    #[serde(default)]
    pub claims: CredentialValue,
}

#[async_trait]
pub trait ExchangeTransport: Send + Sync {
    async fn send(&self, request: OutboundRequest) -> Result<UpstreamResponse, TransportError>;
}

#[async_trait]
impl ExchangeTransport for ProxiedClient {
    async fn send(&self, request: OutboundRequest) -> Result<UpstreamResponse, TransportError> {
        // LiteLLM's httpx client follows redirects, so each hop is followed and re-checked and the body stays capped.
        let response = self.send_following(request).await?;
        response.buffered(self.limits.body_bytes).await
    }
}

pub struct TokenEndpointPost<Transport> {
    pub transport: Transport,
    pub timeout: Duration,
}

fn encode_form(form: &BTreeMap<String, SecretString>) -> String {
    let mut serializer = form_urlencoded::Serializer::new(String::new());
    serializer.extend_pairs(
        form.iter()
            .map(|(name, value)| (name.as_str(), value.expose_secret())),
    );
    serializer.finish()
}

// serde's derived struct also accepts a JSON sequence positionally, so the body must start as an object.
fn parse_object<T>(body: &[u8]) -> Option<T>
where
    T: DeserializeOwned,
{
    let start = body.iter().position(|byte| !byte.is_ascii_whitespace())?;
    if body.get(start) != Some(&b'{') {
        return None;
    }
    serde_json::from_slice(body.get(start..)?).ok()
}

fn body_fields(body: &[u8]) -> (Option<String>, Option<String>) {
    let Some(parsed) = parse_object::<OAuthErrorBody>(body) else {
        return (None, None);
    };
    let error = match &parsed.error {
        CredentialValue::Text(code) => Some(code.clone()),
        CredentialValue::Null
        | CredentialValue::Bool(_)
        | CredentialValue::Number(_)
        | CredentialValue::Array(_)
        | CredentialValue::Object(_) => None,
    };
    let claims = match &parsed.claims {
        CredentialValue::Text(claims) if !claims.is_empty() => Some(claims.clone()),
        CredentialValue::Text(_)
        | CredentialValue::Null
        | CredentialValue::Bool(_)
        | CredentialValue::Number(_)
        | CredentialValue::Array(_)
        | CredentialValue::Object(_) => None,
    };
    (error, claims)
}

fn classify(status: u16, body: &[u8]) -> Result<Option<ExchangeTokenResponse>, ExchangeRejection> {
    if (400..500).contains(&status) {
        let (error, claims) = body_fields(body);
        if let Some(code) = error
            && GATEWAY_FAULT_OAUTH_ERRORS.contains(&code.as_str())
        {
            tracing::warn!(
                %code,
                status,
                "MCP token exchange rejected the gateway's credentials, audience or scope"
            );
            return Err(ExchangeRejection::ClientFault { code });
        }
        return Err(ExchangeRejection::SubjectRejected {
            detail: format!("IdP rejected the subject token (HTTP {status})"),
            claims,
        });
    }
    if !(200..300).contains(&status) {
        tracing::warn!(status, "MCP token exchange request failed");
        return Ok(None);
    }
    let Some(parsed) = parse_object::<ExchangeTokenResponse>(body) else {
        tracing::warn!("MCP token exchange returned a non-object JSON body");
        return Ok(None);
    };
    Ok(Some(parsed))
}

#[async_trait]
impl<Transport: ExchangeTransport> ExchangeHttpPost for TokenEndpointPost<Transport> {
    async fn post(
        &self,
        url: &str,
        form: &BTreeMap<String, SecretString>,
        headers: &BTreeMap<String, SecretString>,
    ) -> Result<Option<ExchangeTokenResponse>, ExchangeRejection> {
        let Ok(parsed_url) = Url::parse(url) else {
            tracing::warn!("MCP token exchange endpoint is not a URL");
            return Ok(None);
        };
        let mut request_headers = HeaderMap::new();
        request_headers.insert(
            HeaderName::from_static("accept"),
            HeaderValue::from_static("application/json"),
        );
        for (header_name, secret_value) in headers {
            let (Ok(valid_name), Ok(mut valid_value)) = (
                HeaderName::try_from(header_name.as_str()),
                HeaderValue::from_str(secret_value.expose_secret()),
            ) else {
                return Ok(None);
            };
            valid_value.set_sensitive(true);
            request_headers.insert(valid_name, valid_value);
        }
        let request = OutboundRequest {
            method: Method::POST,
            url: parsed_url,
            query: Vec::new(),
            headers: request_headers,
            body: RequestBody::Bytes {
                content_type: HeaderValue::from_static("application/x-www-form-urlencoded"),
                data: encode_form(form).into_bytes(),
            },
            deadline: self.timeout,
        };
        let Ok(response) = self.transport.send(request).await else {
            tracing::warn!("MCP token exchange request failed");
            return Ok(None);
        };
        classify(response.status, &response.body)
    }
}

pub type BuiltTokenExchanger<Transport> = OboTokenExchanger<
    TokenEndpointPost<Transport>,
    InMemoryTokenCacheBackend,
    InProcessRefreshCoordinator<Option<OAuthToken>, ExchangeRejection>,
>;

pub fn build_token_exchanger<Transport>(
    transport: Transport,
    tunables: TokenExchangeTunables,
) -> BuiltTokenExchanger<Transport>
where
    Transport: ExchangeTransport,
{
    let clock: Arc<dyn WallClock> = Arc::new(SystemWallClock);
    OboTokenExchanger {
        post: TokenEndpointPost {
            transport,
            timeout: EXCHANGE_TIMEOUT,
        },
        cache: InMemoryTokenCacheBackend {
            max_size: tunables.cache_max_size,
            clock: Arc::clone(&clock),
            entries: TokenCacheEntries::default(),
        },
        coordinator: InProcessRefreshCoordinator::default(),
        clock,
        default_ttl: tunables.default_ttl,
        min_ttl: tunables.min_ttl,
        expiry_buffer: tunables.expiry_buffer,
    }
}
