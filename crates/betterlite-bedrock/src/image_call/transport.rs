use std::fmt::{self, Formatter};
use std::future::Future;
use std::time::Duration;

use betterlite_http::proxy::transport_error::TransportError;
use thiserror::Error;

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
