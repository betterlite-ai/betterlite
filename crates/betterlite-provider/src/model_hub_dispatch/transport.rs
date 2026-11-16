use super::HubDispatchError;
use super::hub::headers_from;
use crate::operation::OperationError;
use async_trait::async_trait;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::{ProxiedClient, StreamResponse, UpstreamResponse};
use betterlite_openai::compatible_wire::header_map;
use betterlite_openai::huggingface_chat::mapping::HubResponse;
use betterlite_openai::openai_base_config::Headers;
use http::Method;
use std::time::Duration;
use url::Url;
pub struct WireRequest<'request> {
    pub method: Method,
    pub url: &'request str,
    pub query: &'request [(&'request str, &'request str)],
    pub headers: &'request Headers,
    pub body: Option<&'request [u8]>,
    pub deadline: Duration,
}
impl WireRequest<'_> {
    pub fn outbound(&self) -> Result<OutboundRequest, HubDispatchError> {
        Ok(OutboundRequest {
            method: self.method.clone(),
            url: Url::parse(self.url).map_err(|source| HubDispatchError::Url {
                url: self.url.to_owned(),
                source,
            })?,
            query: self
                .query
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
            headers: header_map(self.headers)
                .map_err(|source| HubDispatchError::Headers { source })?,
            body: self.body.map_or(RequestBody::Empty, |bytes| {
                RequestBody::Json(bytes.to_vec())
            }),
            deadline: self.deadline,
        })
    }
}
#[async_trait]
pub trait HubGetAsync: Send + Sync {
    async fn get(&self, request: &WireRequest<'_>) -> Result<HubResponse, HubDispatchError>;
}
#[async_trait]
pub trait PostTransport: Send + Sync {
    async fn post(&self, request: &WireRequest<'_>) -> Result<UpstreamResponse, HubDispatchError>;
    async fn post_stream(
        &self,
        request: &WireRequest<'_>,
    ) -> Result<StreamResponse, HubDispatchError>;
}
pub struct Pooled<'client> {
    pub client: &'client ProxiedClient,
}
#[async_trait]
impl HubGetAsync for Pooled<'_> {
    async fn get(&self, request: &WireRequest<'_>) -> Result<HubResponse, HubDispatchError> {
        let response = self
            .client
            .send_buffered(request.outbound()?)
            .await
            .map_err(OperationError::from)?;
        Ok(HubResponse {
            status_code: response.status,
            headers: headers_from(&response.headers),
            body: response.body,
        })
    }
}
#[async_trait]
impl PostTransport for Pooled<'_> {
    async fn post(&self, request: &WireRequest<'_>) -> Result<UpstreamResponse, HubDispatchError> {
        self.client
            .send_buffered(request.outbound()?)
            .await
            .map_err(|source| HubDispatchError::Operation(OperationError::from(source)))
    }
    async fn post_stream(
        &self,
        request: &WireRequest<'_>,
    ) -> Result<StreamResponse, HubDispatchError> {
        self.client
            .send(request.outbound()?)
            .await
            .map_err(|source| HubDispatchError::Operation(OperationError::from(source)))
    }
}
