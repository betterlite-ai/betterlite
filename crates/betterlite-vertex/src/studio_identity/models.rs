use std::future::Future;
use std::time::Duration;

use betterlite_config::sections::studio_identity::Settings;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use http::Method;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

use super::API_VERSION;
use super::decode::decode_studio_models;
use super::error::StudioIdentityError;

#[derive(Debug)]
pub struct ModelsRequest {
    pub url: String,
    pub api_key: SecretString,
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

impl ModelsRequest {
    pub fn into_outbound(self, deadline: Duration) -> Result<OutboundRequest, StudioIdentityError> {
        let url =
            Url::parse(&self.url).map_err(|source| StudioIdentityError::InvalidUrl { source })?;
        let mut headers = HeaderMap::new();
        let mut api_key =
            HeaderValue::from_str(self.api_key.expose_secret()).map_err(|source| {
                StudioIdentityError::InvalidHeader {
                    name: "x-goog-api-key",
                    source,
                }
            })?;
        api_key.set_sensitive(true);
        headers.insert(HeaderName::from_static("x-goog-api-key"), api_key);
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
    settings: &Settings,
) -> Result<ModelsRequest, StudioIdentityError> {
    let key = api_key
        .cloned()
        .or_else(|| settings.environment_key.clone())
        .ok_or(StudioIdentityError::MissingCredentials)?;
    let base = api_base.unwrap_or(&settings.api_base);
    Ok(ModelsRequest {
        url: format!("{base}/{API_VERSION}/models"),
        api_key: key,
    })
}

pub async fn get_models<T>(
    transport: &T,
    api_key: Option<&SecretString>,
    api_base: Option<&str>,
    settings: &Settings,
    deadline: Duration,
) -> Result<Vec<String>, StudioIdentityError>
where
    T: ModelsTransport,
{
    let request = models_request(api_key, api_base, settings)?.into_outbound(deadline)?;
    let reply = transport
        .send(request)
        .await
        .map_err(|source| StudioIdentityError::Transport { source })?;
    decode_studio_models(reply.status, &reply.body)
}
