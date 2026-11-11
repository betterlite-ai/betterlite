use std::fmt::{self, Formatter};
use std::future::Future;
use std::str;
use std::time::Duration;

use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::Method;
use http::header::{HeaderMap, HeaderName, HeaderValue, InvalidHeaderName};
use thiserror::Error;
use url::Url;

#[derive(Clone, PartialEq, Eq)]
pub struct ImageRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

// the signed headers carry the provider credential, so Debug names them without their values
impl fmt::Debug for ImageRequest {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let names: Vec<&str> = self.headers.iter().map(|(name, _)| name.as_str()).collect();
        f.debug_struct("ImageRequest")
            .field("url", &self.url)
            .field("header_names", &names)
            .field("body_length", &self.body.len())
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageReply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Debug, Error)]
pub enum TransferError {
    #[error("the provider request timed out")]
    Timeout,
    #[error("the provider URL is invalid")]
    Url(#[from] url::ParseError),
    #[error("the {name} header name is invalid")]
    HeaderName {
        name: String,
        #[source]
        source: InvalidHeaderName,
    },
    #[error(transparent)]
    Failed(#[from] TransportError),
}

pub trait ImageTransport: Send + Sync {
    fn post(
        &self,
        request: ImageRequest,
    ) -> impl Future<Output = Result<ImageReply, TransferError>> + Send;
}

// the supplied client's kind decides whether LiteLLM accepts it for the sync or the async path
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientKind {
    Sync,
    Async,
}

pub struct SuppliedClient<'client, Client> {
    pub kind: ClientKind,
    pub transport: &'client Client,
}

// B173 owns client construction; this seam receives the sync client or the async client built with the call timeout
pub trait ClientFactory {
    type Client: ImageTransport;

    fn sync_client(&self) -> Self::Client;

    fn async_client(&self, timeout: Option<Duration>) -> Self::Client;
}

impl ImageTransport for ProxiedClient {
    async fn post(&self, request: ImageRequest) -> Result<ImageReply, TransferError> {
        let outbound = OutboundRequest {
            method: Method::POST,
            url: Url::parse(&request.url)?,
            query: Vec::new(),
            headers: header_map(&request.headers)?,
            body: RequestBody::Bytes {
                content_type: HeaderValue::from_static("application/json"),
                data: request.body,
            },
            deadline: self.limits.read_timeout,
        };
        let response = self.send_buffered(outbound).await.map_err(transfer_error)?;
        Ok(ImageReply {
            status: response.status,
            headers: reply_headers(&response.headers),
            body: response.body,
        })
    }
}

fn header_map(headers: &[(String, String)]) -> Result<HeaderMap, TransferError> {
    let mut mapped = HeaderMap::new();
    for (name, value) in headers {
        let header = HeaderName::from_bytes(name.as_bytes()).map_err(|source| {
            TransferError::HeaderName {
                name: name.clone(),
                source,
            }
        })?;
        let header_value = HeaderValue::from_str(value).map_err(|source| {
            TransferError::Failed(TransportError::InvalidHeader {
                name: name.clone(),
                source,
            })
        })?;
        mapped.insert(header, header_value);
    }
    Ok(mapped)
}

// httpx hands a caller the header text as it arrived, so a not-UTF-8 value keeps its latin-1 characters
fn reply_headers(headers: &HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| (name.as_str().to_owned(), header_text(value)))
        .collect()
}

fn header_text(value: &HeaderValue) -> String {
    str::from_utf8(value.as_bytes()).map_or_else(
        |_| value.as_bytes().iter().copied().map(char::from).collect(),
        str::to_owned,
    )
}

// httpx raises TimeoutException for the two timeout failures and lets every other one propagate
fn transfer_error(error: TransportError) -> TransferError {
    if matches!(
        error,
        TransportError::ConnectTimeout(_) | TransportError::Deadline(_)
    ) {
        TransferError::Timeout
    } else {
        TransferError::Failed(error)
    }
}
