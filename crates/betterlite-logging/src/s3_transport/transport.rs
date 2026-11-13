use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use http::Method;
use http::header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};

use crate::s3_transport::encryption::ServerSideEncryption;
use crate::s3_transport::error::S3TransportError;
use crate::s3_transport::signing::{Clock, CredentialSource};
use crate::s3_transport::url_construction::S3Endpoint;

mod tests;

pub const S3_REQUEST_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UploadOutcome {
    Delivered,
    Retry,
    Dropped,
}

#[derive(Clone, Debug)]
pub struct S3Request {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct S3Response {
    pub status: u16,
    pub body: Vec<u8>,
}

pub trait S3HttpClient: Send + Sync + Debug {
    fn send(
        &self,
        request: S3Request,
    ) -> Pin<Box<dyn Future<Output = Result<S3Response, S3TransportError>> + Send + '_>>;
}

pub trait ThrottleSink: Send + Sync + Debug {
    fn record(&self, throttled: bool);
}

#[derive(Clone, Debug)]
pub struct HttpS3Client {
    pub client: ProxiedClient,
}

impl S3HttpClient for HttpS3Client {
    fn send(
        &self,
        request: S3Request,
    ) -> Pin<Box<dyn Future<Output = Result<S3Response, S3TransportError>> + Send + '_>> {
        Box::pin(async move {
            let url = url::Url::parse(&request.url).map_err(|source| S3TransportError::Url {
                url: request.url.clone(),
                source,
            })?;
            let headers = header_map(&request.headers)?;
            let content_type = headers
                .get(CONTENT_TYPE)
                .cloned()
                .unwrap_or(HeaderValue::from_static("application/json"));
            let body = if request.body.is_empty() {
                RequestBody::Empty
            } else {
                RequestBody::Bytes {
                    content_type,
                    data: request.body,
                }
            };
            let outbound = OutboundRequest {
                method: request.method,
                url,
                query: Vec::new(),
                headers,
                body,
                deadline: S3_REQUEST_TIMEOUT,
            };
            let response = self.client.send_buffered(outbound).await?;
            Ok(S3Response {
                status: response.status,
                body: response.body,
            })
        })
    }
}

fn header_map(headers: &[(String, String)]) -> Result<HeaderMap, S3TransportError> {
    let mut map = HeaderMap::with_capacity(headers.len());
    for (name, value) in headers {
        let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|source| {
            S3TransportError::HeaderName {
                name: name.clone(),
                source,
            }
        })?;
        let header_value =
            HeaderValue::from_str(value).map_err(|source| S3TransportError::HeaderValue {
                name: name.clone(),
                source,
            })?;
        map.insert(header_name, header_value);
    }
    Ok(map)
}

#[derive(Debug)]
pub struct S3Transport {
    pub client: Arc<dyn S3HttpClient>,
    pub endpoint: S3Endpoint,
    pub credentials: Arc<dyn CredentialSource>,
    pub encryption: Option<ServerSideEncryption>,
    pub drop_on_terminal_error: bool,
    pub throttle: Option<Arc<dyn ThrottleSink>>,
    pub clock: Arc<dyn Clock>,
}
