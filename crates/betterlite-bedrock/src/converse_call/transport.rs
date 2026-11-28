use std::sync::Arc;
use std::time::Duration;

use crate::agentcore_stream::transport::StreamHandler;
use crate::rerank::transport::RerankTransport;
use crate::signing::endpoint::{RequestHeaders, request_headers};
use crate::signing::error::SigningError;
use crate::signing::request::AwsSignedRequest;
use betterlite_core::api::vector_store_http_write::service::ClientKind;

// B595's get_request_headers behind a seam, so a call can be asked what it would sign.
pub trait ConverseSigner: Sync {
    fn sign(&self, request: RequestHeaders<'_>) -> Result<AwsSignedRequest, SigningError>;
}

pub struct SigV4Signer;

impl ConverseSigner for SigV4Signer {
    fn sign(&self, request: RequestHeaders<'_>) -> Result<AwsSignedRequest, SigningError> {
        request_headers(request)
    }
}

// The streaming client LiteLLM's make_call receives: the handler and the timeout it was built with.
#[derive(Clone)]
pub struct StreamClient {
    pub handler: Arc<dyn StreamHandler>,
    pub timeout: Duration,
}

pub struct SuppliedStream {
    pub kind: ClientKind,
    pub client: StreamClient,
}

// B173 owns client construction; a call asks for the sync or async default it should use.
pub trait ConverseClientFactory: Sync {
    type Buffered: RerankTransport;

    fn buffered(&self, kind: ClientKind, timeout: Option<Duration>) -> Self::Buffered;

    // The async stream client never receives the call timeout; only the sync one is built with it.
    fn stream(&self, kind: ClientKind, timeout: Option<Duration>) -> StreamClient;
}
