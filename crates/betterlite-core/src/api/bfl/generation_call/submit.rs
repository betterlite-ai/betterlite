use std::future::Future;
use std::time::Duration;

use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use betterlite_models::decisions::Json;
use http::Method;
use http::header::HeaderMap;
use url::Url;

#[derive(Debug, thiserror::Error)]
pub enum SubmitError {
    #[error("the BFL URL is not a URL")]
    Url(#[source] url::ParseError),
    #[error("the BFL request body could not be serialized")]
    Body(#[source] serde_json::Error),
    #[error(transparent)]
    Transport(#[from] TransportError),
}

pub struct SubmitRequest {
    pub url: Url,
    pub headers: HeaderMap,
    pub body: Json,
    pub timeout: Option<Duration>,
}

pub trait SubmitTransport {
    fn post(
        &self,
        request: SubmitRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, SubmitError>> + Send;
}

// LiteLLM hands the caller timeout to the submission request and lets the client's own timeout stand when there is none
impl SubmitTransport for ProxiedClient {
    async fn post(&self, request: SubmitRequest) -> Result<UpstreamResponse, SubmitError> {
        let body = serde_json::to_vec(&request.body).map_err(SubmitError::Body)?;
        self.send_buffered(OutboundRequest {
            method: Method::POST,
            url: request.url,
            query: Vec::new(),
            headers: request.headers,
            body: RequestBody::Json(body),
            deadline: request.timeout.unwrap_or(self.limits.read_timeout),
        })
        .await
        .map_err(SubmitError::Transport)
    }
}
