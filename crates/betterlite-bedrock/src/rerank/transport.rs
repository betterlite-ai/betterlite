use std::fmt::{self, Debug, Formatter};
use std::future::Future;
use std::time::Duration;

use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_state::LoggingState;
use thiserror::Error;

#[derive(Clone, PartialEq, Eq)]
pub struct RerankHttpRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub timeout: Option<Duration>,
}

// the signed headers carry the provider credential, so Debug names them without their values
impl Debug for RerankHttpRequest {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let names: Vec<&str> = self.headers.iter().map(|(name, _)| name.as_str()).collect();
        f.debug_struct("RerankHttpRequest")
            .field("url", &self.url)
            .field("header_names", &names)
            .field("body_length", &self.body.len())
            .field("timeout", &self.timeout)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RerankHttpReply {
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

pub trait RerankTransport: Send + Sync {
    fn post(
        &self,
        request: RerankHttpRequest,
    ) -> impl Future<Output = Result<RerankHttpReply, TransferError>> + Send;

    // LiteLLM hands the async client its logging object and the sync client none
    fn post_with_logging(
        &self,
        request: RerankHttpRequest,
        logging: &LoggingState,
    ) -> impl Future<Output = Result<RerankHttpReply, TransferError>> + Send;
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

// B173 owns client construction; the factory answers the sync default or the async BEDROCK client
pub trait ClientFactory {
    type Client: RerankTransport;

    fn sync_client(&self) -> Self::Client;

    fn async_client(&self) -> Self::Client;
}
