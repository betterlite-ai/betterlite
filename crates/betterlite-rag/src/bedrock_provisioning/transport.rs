use std::fmt::{Debug, Formatter, Result as FmtResult};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_bedrock::signing::request::AwsSignedRequest;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::Method;
use http::header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use url::Url;

use crate::bedrock_ingestion::client::{AwsRagResponse, AwsRagTransport};

pub const REQUEST_DEADLINE: Duration = Duration::from_secs(60);
const FALLBACK_CONTENT_TYPE: &str = "application/octet-stream";

#[derive(Debug, thiserror::Error)]
pub enum HttpRagTransportError {
    #[error("the AWS request method {method} is not an HTTP method")]
    Method { method: String },
    #[error("the AWS request URL {url} cannot be parsed")]
    Url { url: String },
    #[error("the AWS request header {name} cannot be sent")]
    Header { name: String },
    #[error("the AWS request could not be sent")]
    Transport {
        #[source]
        source: TransportError,
    },
}

pub struct HttpRagTransport {
    pub client: Arc<ProxiedClient>,
}

impl Debug for HttpRagTransport {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.debug_struct("HttpRagTransport").finish_non_exhaustive()
    }
}

#[async_trait]
impl AwsRagTransport for HttpRagTransport {
    type Error = HttpRagTransportError;

    async fn send(&self, request: AwsSignedRequest) -> Result<AwsRagResponse, Self::Error> {
        let method = Method::from_bytes(request.method.as_bytes()).map_err(|_source| {
            HttpRagTransportError::Method {
                method: request.method.clone(),
            }
        })?;
        let url = Url::parse(&request.url).map_err(|_source| HttpRagTransportError::Url {
            url: request.url.clone(),
        })?;
        let headers = header_map(&request.headers)?;
        let content_type = headers
            .get(CONTENT_TYPE)
            .cloned()
            .unwrap_or_else(|| HeaderValue::from_static(FALLBACK_CONTENT_TYPE));
        let body = if request.body.is_empty() {
            RequestBody::Empty
        } else {
            RequestBody::Bytes {
                content_type,
                data: request.body,
            }
        };
        let outbound = OutboundRequest {
            method,
            url,
            query: Vec::new(),
            headers,
            body,
            deadline: REQUEST_DEADLINE,
        };
        let response = self
            .client
            .send_buffered(outbound)
            .await
            .map_err(|source| HttpRagTransportError::Transport { source })?;
        Ok(AwsRagResponse {
            status: response.status,
            body: response.body,
        })
    }
}

fn header_map(headers: &[(String, String)]) -> Result<HeaderMap, HttpRagTransportError> {
    let mut map = HeaderMap::with_capacity(headers.len());
    for (name, value) in headers {
        let parsed_name = HeaderName::try_from(name.as_str())
            .map_err(|_source| HttpRagTransportError::Header { name: name.clone() })?;
        let parsed_value = HeaderValue::try_from(value.as_str())
            .map_err(|_source| HttpRagTransportError::Header { name: name.clone() })?;
        let _previous = map.insert(parsed_name, parsed_value);
    }
    Ok(map)
}

mod tests;
