use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::{ProxiedClient, StreamResponse, UpstreamResponse};
use betterlite_openai::compatible_wire::header_map;
use betterlite_openai::openai_base_config::Headers;
use http::Method;
use url::Url;

use super::HubDispatchError;
use crate::operation::OperationError;

// the POSTs the embedding and Predibase arms send; the pooled client backs them in service and a scripted answer in tests
#[async_trait]
pub trait PostTransport: Send + Sync {
    async fn post(
        &self,
        url: &str,
        headers: &Headers,
        body: &[u8],
        deadline: Duration,
    ) -> Result<UpstreamResponse, HubDispatchError>;

    async fn post_stream(
        &self,
        url: &str,
        headers: &Headers,
        body: &[u8],
        deadline: Duration,
    ) -> Result<StreamResponse, HubDispatchError>;
}

pub struct PooledPost<'client> {
    pub client: &'client ProxiedClient,
}

impl PooledPost<'_> {
    fn request(
        &self,
        url: &str,
        headers: &Headers,
        body: &[u8],
        deadline: Duration,
    ) -> Result<OutboundRequest, HubDispatchError> {
        Ok(OutboundRequest {
            method: Method::POST,
            url: Url::parse(url).map_err(|source| HubDispatchError::Url {
                url: url.to_owned(),
                source,
            })?,
            query: Vec::new(),
            headers: header_map(headers).map_err(|source| HubDispatchError::Headers { source })?,
            body: RequestBody::Json(body.to_vec()),
            deadline,
        })
    }
}

#[async_trait]
impl PostTransport for PooledPost<'_> {
    async fn post(
        &self,
        url: &str,
        headers: &Headers,
        body: &[u8],
        deadline: Duration,
    ) -> Result<UpstreamResponse, HubDispatchError> {
        self.client
            .send_buffered(self.request(url, headers, body, deadline)?)
            .await
            .map_err(OperationError::from)
            .map_err(HubDispatchError::Operation)
    }

    async fn post_stream(
        &self,
        url: &str,
        headers: &Headers,
        body: &[u8],
        deadline: Duration,
    ) -> Result<StreamResponse, HubDispatchError> {
        self.client
            .send(self.request(url, headers, body, deadline)?)
            .await
            .map_err(OperationError::from)
            .map_err(HubDispatchError::Operation)
    }
}
