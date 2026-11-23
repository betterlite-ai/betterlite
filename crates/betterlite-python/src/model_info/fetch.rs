use super::error::ModelInfoError;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_models::fetch_policy::client::{
    CatalogClient, CatalogClientError, CatalogRequest, FetchErrorKind,
};
use betterlite_models::fetch_policy::response::FetchResponse;
use betterlite_models::lemonade_model_metadata::{ModelInfoClient, ModelInfoRequest};
use http::{HeaderMap, HeaderName, HeaderValue, Method};
use secrecy::ExposeSecret as _;
use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use url::Url;
// One outbound call path for the cost map fetch and the Hugging Face config lookup, both reused from betterlite-http.
#[derive(Clone)]
pub(crate) struct HttpFetch {
    client: ProxiedClient,
}
impl HttpFetch {
    pub(crate) fn new(environment: &BTreeMap<String, String>) -> Result<Self, ModelInfoError> {
        let policy = ProxyPolicy::resolve(&ProxyTrust::default(), environment)
            .map_err(|source| ModelInfoError::Client(source.to_string()))?;
        let client = ProxiedClient::outbound(&Arc::new(policy))
            .map_err(|source| ModelInfoError::Client(source.to_string()))?;
        Ok(Self { client })
    }
    async fn send(
        &self,
        url: &str,
        timeout: Duration,
        headers: HeaderMap,
    ) -> Result<FetchResponse, FetchError> {
        let parsed =
            Url::parse(url).map_err(|source| FetchError::InvalidUrl(source.to_string()))?;
        let request = OutboundRequest {
            method: Method::GET,
            url: parsed,
            query: Vec::new(),
            headers,
            body: RequestBody::Empty,
            deadline: timeout,
        };
        let response = self
            .client
            .send_buffered(request)
            .await
            .map_err(|source| FetchError::Transport(source.to_string()))?;
        Ok(FetchResponse::new(
            response.status,
            header_pairs(&response.headers),
            response.body,
        ))
    }
}
fn header_pairs(headers: &HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|text| (name.as_str().to_owned(), text.to_owned()))
        })
        .collect()
}
#[derive(Debug, thiserror::Error)]
pub(crate) enum FetchError {
    #[error("the URL {0} is not valid")]
    InvalidUrl(String),
    #[error("the request failed: {0}")]
    Transport(String),
    #[error("a header is not valid: {0}")]
    Header(String),
    #[error("no blocking HTTP client is available")]
    Blocking,
}
impl CatalogClientError for FetchError {
    fn kind(&self) -> FetchErrorKind {
        match self {
            Self::InvalidUrl(_) => FetchErrorKind::InvalidUrl,
            Self::Transport(_) | Self::Header(_) | Self::Blocking => FetchErrorKind::Transport,
        }
    }
}
impl CatalogClient for HttpFetch {
    type Error = FetchError;
    // The reload entry points run inside the async runtime, so the blocking trait method is never used.
    fn get(&self, _request: &CatalogRequest) -> Result<FetchResponse, Self::Error> {
        Err(FetchError::Blocking)
    }
    fn get_async(
        &self,
        request: &CatalogRequest,
    ) -> impl Future<Output = Result<FetchResponse, Self::Error>> + Send {
        let client = self.clone();
        let url = request.url.clone();
        let timeout = request.timeout;
        async move { client.send(&url, timeout, HeaderMap::new()).await }
    }
}
impl ModelInfoClient for HttpFetch {
    type Error = FetchError;
    fn get(
        &self,
        request: &ModelInfoRequest,
    ) -> impl Future<Output = Result<FetchResponse, Self::Error>> + Send {
        let client = self.clone();
        let url = request.url.clone();
        let timeout = request.timeout;
        let headers = request.headers.clone();
        async move {
            let mut map = HeaderMap::new();
            for (name, value) in &headers {
                let header_name = HeaderName::from_bytes(name.as_bytes())
                    .map_err(|source| FetchError::Header(source.to_string()))?;
                let header_value = HeaderValue::from_str(value.expose_secret())
                    .map_err(|source| FetchError::Header(source.to_string()))?;
                map.insert(header_name, header_value);
            }
            client.send(&url, timeout, map).await
        }
    }
}
