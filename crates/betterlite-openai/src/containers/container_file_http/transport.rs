use std::collections::BTreeMap;
use std::future::Future;

use betterlite_http::proxy::pool::PoolError;
use betterlite_http::proxy::transport_error::TransportError;
use http::header::InvalidHeaderName;
use thiserror::Error;

use super::request::SslVerify;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerMethod {
    Get,
    Delete,
    Post,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadedFile {
    pub filename: String,
    pub content: Vec<u8>,
    pub content_type: String,
}

// There is no timeout field: the selected client keeps its own deadline, as in LiteLLM
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerHttpRequest {
    pub method: ContainerMethod,
    pub url: String,
    pub headers: BTreeMap<String, String>,
    // empty means no params at all, so the query string already in the URL stays
    pub query: Vec<(String, String)>,
    pub file: Option<UploadedFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerHttpReply {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

#[derive(Debug, Error)]
pub enum TransferError {
    #[error(transparent)]
    Transport(#[from] TransportError),
    #[error("the request URL {url} is not valid")]
    Url {
        url: String,
        #[source]
        source: url::ParseError,
    },
    #[error("{name} is not a valid header name")]
    HeaderName {
        name: String,
        #[source]
        source: InvalidHeaderName,
    },
}

pub trait BlockingClient: Send + Sync {
    fn send(&self, request: &ContainerHttpRequest) -> Result<ContainerHttpReply, TransferError>;
}

pub trait AsyncClient: Send + Sync {
    fn send(
        &self,
        request: &ContainerHttpRequest,
    ) -> impl Future<Output = Result<ContainerHttpReply, TransferError>> + Send;
}

// B173 owns the pool behind this: the blocking and async clients are different types, as HTTPHandler and AsyncHTTPHandler are
pub trait ClientSource: Send + Sync {
    type Blocking: BlockingClient;
    type Asynchronous: AsyncClient;

    fn blocking_client(&self, ssl_verify: Option<&SslVerify>) -> Result<Self::Blocking, PoolError>;

    fn async_client(&self, ssl_verify: Option<&SslVerify>)
    -> Result<Self::Asynchronous, PoolError>;
}

// A supplied client is reused only when its kind matches the method called
pub enum SuppliedClient<'client, Source: ClientSource> {
    Blocking(&'client Source::Blocking),
    Asynchronous(&'client Source::Asynchronous),
}
