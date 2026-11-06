use std::collections::BTreeMap;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use http::Method;
use http::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde::de::IgnoredAny;
use url::Url;
use url::form_urlencoded::Serializer;

use super::config::DatabricksAppOAuthConfig;
use super::error::DatabricksOauthError;
use super::expires_in::{ExpiresIn, cache_ttl};

// LiteLLM's default httpx read timeout for its cached clients.
const TOKEN_REQUEST_DEADLINE: Duration = Duration::from_secs(600);

#[derive(Deserialize)]
#[serde(untagged)]
enum AccessToken {
    Text(String),
    Other(IgnoredAny),
}

#[derive(Deserialize)]
struct TokenResponse {
    #[serde(default)]
    access_token: Option<AccessToken>,
    #[serde(default)]
    expires_in: Option<ExpiresIn>,
    // flatten makes serde accept only a JSON object, never an array read field by field
    #[serde(flatten)]
    _rest: BTreeMap<String, ExpiresIn>,
}

fn token_request(
    config: &DatabricksAppOAuthConfig,
) -> Result<OutboundRequest, DatabricksOauthError> {
    let url = Url::parse(&config.token_url).map_err(DatabricksOauthError::TokenUrl)?;
    let credentials = format!(
        "{}:{}",
        config.client_id,
        config.client_secret.expose_secret()
    );
    let mut authorization =
        HeaderValue::try_from(format!("Basic {}", STANDARD.encode(credentials)))
            .map_err(DatabricksOauthError::CredentialHeader)?;
    authorization.set_sensitive(true);
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, authorization);
    let form = Serializer::new(String::new())
        .append_pair("grant_type", "client_credentials")
        .append_pair("scope", &config.scope)
        .finish();
    Ok(OutboundRequest {
        method: Method::POST,
        url,
        query: Vec::new(),
        headers,
        body: RequestBody::Bytes {
            content_type: HeaderValue::from_static("application/x-www-form-urlencoded"),
            data: form.into_bytes(),
        },
        deadline: TOKEN_REQUEST_DEADLINE,
    })
}

pub async fn fetch_token(
    client: &ProxiedClient,
    config: &DatabricksAppOAuthConfig,
) -> Result<(SecretString, u32), DatabricksOauthError> {
    let request = token_request(config)?;
    tracing::debug!(token_url = %config.token_url, "fetching Databricks App OAuth token");
    let response = client
        .send_buffered(request)
        .await
        .map_err(DatabricksOauthError::TokenRequest)?;
    if !(200..300).contains(&response.status) {
        return Err(DatabricksOauthError::TokenStatus {
            status: response.status,
        });
    }
    let parsed: TokenResponse =
        serde_json::from_slice(&response.body).map_err(DatabricksOauthError::TokenResponse)?;
    match parsed.access_token {
        Some(AccessToken::Text(token)) if !token.is_empty() => Ok((
            SecretString::from(token),
            cache_ttl(parsed.expires_in.as_ref()),
        )),
        Some(AccessToken::Text(_) | AccessToken::Other(_)) | None => {
            Err(DatabricksOauthError::MissingAccessToken)
        }
    }
}
