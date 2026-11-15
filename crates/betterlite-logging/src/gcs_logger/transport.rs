use std::time::{Duration, Instant};

use async_trait::async_trait;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use http::Method;
use http::header::{CONTENT_TYPE, HeaderValue};
use thiserror::Error;
use url::Url;

use crate::gcs_mock::client::{GcsHttpClient, GcsHttpResponse};
use crate::gcs_mock::request::{GcsRequest, GcsWriteRequest};

// the storage loggers share one upload deadline, so a slow bucket fails the batch instead of holding the queue
const GCS_REQUEST_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Debug, Error)]
pub enum GcsTransportError {
    #[error("the GCS object URL {url} is not valid")]
    Url {
        url: String,
        #[source]
        source: url::ParseError,
    },
    #[error("the GCS logging payload could not be serialized")]
    Payload(#[source] serde_json::Error),
    #[error("the GCS request could not be sent")]
    Send(#[source] TransportError),
}

pub struct PooledGcsClient {
    pub client: ProxiedClient,
}

#[async_trait]
impl GcsHttpClient for PooledGcsClient {
    type Error = GcsTransportError;

    async fn send(&self, request: GcsRequest) -> Result<GcsHttpResponse, Self::Error> {
        let started = Instant::now();
        let url_text = request.url().to_owned();
        let url = Url::parse(&url_text).map_err(|source| GcsTransportError::Url {
            url: url_text.clone(),
            source,
        })?;
        let (method, write) = match request {
            GcsRequest::Get(read) => {
                let response = self
                    .client
                    .send_buffered(OutboundRequest {
                        method: Method::GET,
                        url,
                        query: read.params.into_iter().collect(),
                        headers: read.headers,
                        body: RequestBody::Empty,
                        deadline: GCS_REQUEST_TIMEOUT,
                    })
                    .await
                    .map_err(GcsTransportError::Send)?;
                return Ok(response_of(response, url_text, started));
            }
            GcsRequest::Post(inner) => (Method::POST, inner),
            GcsRequest::Delete(inner) => (Method::DELETE, inner),
        };
        let (body, deadline) = write_body(&write)?;
        let response = self
            .client
            .send_buffered(OutboundRequest {
                method,
                url,
                query: write.params.into_iter().collect(),
                headers: write.headers,
                body,
                deadline,
            })
            .await
            .map_err(GcsTransportError::Send)?;
        Ok(response_of(response, url_text, started))
    }
}

fn write_body(request: &GcsWriteRequest) -> Result<(RequestBody, Duration), GcsTransportError> {
    let deadline = request.timeout.unwrap_or(GCS_REQUEST_TIMEOUT);
    if let Some(content) = request.content.as_ref() {
        let content_type = request
            .headers
            .get(CONTENT_TYPE)
            .cloned()
            .unwrap_or(HeaderValue::from_static("application/json"));
        return Ok((
            RequestBody::Bytes {
                content_type,
                data: content.clone(),
            },
            deadline,
        ));
    }
    if let Some(json) = request.json.as_ref() {
        let data = serde_json::to_vec(json).map_err(GcsTransportError::Payload)?;
        return Ok((RequestBody::Json(data), deadline));
    }
    if let Some(data) = request.data.as_ref() {
        let bytes = match data {
            Json::Text(text) => text.clone().into_bytes(),
            other => serde_json::to_vec(other).map_err(GcsTransportError::Payload)?,
        };
        return Ok((
            RequestBody::Bytes {
                content_type: HeaderValue::from_static("application/json"),
                data: bytes,
            },
            deadline,
        ));
    }
    Ok((RequestBody::Empty, deadline))
}

fn response_of(
    response: betterlite_http::proxy::UpstreamResponse,
    url: String,
    started: Instant,
) -> GcsHttpResponse {
    GcsHttpResponse {
        status: response.status,
        headers: response.headers,
        body: response.body,
        url,
        elapsed: started.elapsed(),
    }
}
