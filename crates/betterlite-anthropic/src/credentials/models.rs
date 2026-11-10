use std::future::Future;
use std::time::Duration;

use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use http::{HeaderMap, HeaderName, HeaderValue, Method};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use url::Url;

use super::{AnthropicSettings, AuthHeader, CredentialsError, get_auth_header};

pub const MODEL_LIST_LIMIT: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct ModelsRequest {
    pub url: String,
    pub anthropic_version: String,
    pub auth: AuthHeader,
}

pub trait ModelsTransport: Send + Sync {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send;
}

impl ModelsTransport for ProxiedClient {
    async fn send(&self, request: OutboundRequest) -> Result<UpstreamResponse, TransportError> {
        self.send_buffered(request).await
    }
}

#[derive(Deserialize)]
struct ModelList {
    data: Vec<ModelEntry>,
}

#[derive(Deserialize)]
struct ModelEntry {
    id: String,
}

impl ModelsRequest {
    pub fn into_outbound(self, deadline: Duration) -> Result<OutboundRequest, CredentialsError> {
        let url =
            Url::parse(&self.url).map_err(|source| CredentialsError::InvalidUrl { source })?;
        let mut headers = HeaderMap::new();
        let version = HeaderValue::from_str(&self.anthropic_version).map_err(|_invalid| {
            CredentialsError::InvalidHeader {
                name: "anthropic-version",
            }
        })?;
        headers.insert(HeaderName::from_static("anthropic-version"), version);
        let mut secret =
            HeaderValue::from_str(self.auth.value.expose_secret()).map_err(|_invalid| {
                CredentialsError::InvalidHeader {
                    name: self.auth.name,
                }
            })?;
        secret.set_sensitive(true);
        headers.insert(HeaderName::from_static(self.auth.name), secret);
        Ok(OutboundRequest {
            method: Method::GET,
            url,
            query: Vec::new(),
            headers,
            body: RequestBody::Empty,
            deadline,
        })
    }
}

pub fn models_request(
    api_key: Option<&SecretString>,
    api_base: Option<&str>,
    settings: &AnthropicSettings,
) -> Result<ModelsRequest, CredentialsError> {
    let base = api_base.unwrap_or(&settings.api_base);
    let auth = get_auth_header(api_key, Some(base), false, settings)
        .ok_or(CredentialsError::MissingCredential)?;
    Ok(ModelsRequest {
        url: format!("{base}/v1/models"),
        anthropic_version: settings.anthropic_version.clone(),
        auth,
    })
}

pub async fn get_models<T>(
    transport: &T,
    api_key: Option<&SecretString>,
    api_base: Option<&str>,
    settings: &AnthropicSettings,
    deadline: Duration,
) -> Result<Vec<String>, CredentialsError>
where
    T: ModelsTransport,
{
    let request = models_request(api_key, api_base, settings)?.into_outbound(deadline)?;
    let reply = transport
        .send(request)
        .await
        .map_err(|source| CredentialsError::Transport { source })?;
    decode_anthropic_models(reply.status, &reply.body)
}

pub fn decode_anthropic_models(status: u16, bytes: &[u8]) -> Result<Vec<String>, CredentialsError> {
    if bytes.len() > MODEL_LIST_LIMIT {
        return Err(CredentialsError::ResponseTooLarge {
            limit: MODEL_LIST_LIMIT,
        });
    }
    if !(200..300).contains(&status) {
        let response = str::from_utf8(bytes).map_or_else(
            |_| format!("{} bytes that are not UTF-8", bytes.len()),
            str::to_owned,
        );
        return Err(CredentialsError::ModelsStatus { status, response });
    }
    let list: ModelList =
        serde_json::from_slice(bytes).map_err(|source| CredentialsError::ModelsJson { source })?;
    Ok(list
        .data
        .into_iter()
        .map(|model| format!("anthropic/{}", model.id))
        .collect())
}
