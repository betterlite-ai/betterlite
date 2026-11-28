use std::collections::VecDeque;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_bedrock::bedrock_embedding_io::transport::{
    EmbeddingReply, EmbeddingRequest, EmbeddingTransport, ReplyRequest,
    TransferError as EmbeddingTransferError,
};
use betterlite_bedrock::count_call::handler::CountTokensTransport;
use betterlite_bedrock::embedding_jobs::transport::{
    StatusReply, StatusTransport, StatusWireRequest,
};
use betterlite_bedrock::image_call::transport::{
    ImageReply, ImageRequest, ImageTransport, TransferError as ImageTransferError,
};
use betterlite_bedrock::rerank::transport::{
    RerankHttpReply, RerankHttpRequest, RerankTransport, TransferError as RerankTransferError,
};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_state::LoggingState;
use betterlite_openai::cohere_legacy_embedding::execute::LegacyEmbeddingTransport;
use betterlite_openai::transcription::transport::TranscriptionTransport;

use super::record::{Recorded, Seen, header_map, recorded};

// What the fake upstream answers: a reply, a timeout, or a closed transport
#[derive(Clone, Debug)]
pub enum Reply {
    Answer {
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    },
    Timeout,
    Failed,
}

impl Reply {
    #[must_use]
    pub fn status(status: u16, body: &str) -> Self {
        Self::Answer {
            status,
            headers: Vec::new(),
            body: body.as_bytes().to_vec(),
        }
    }

    #[must_use]
    pub fn with_headers(status: u16, headers: &[(&str, &str)], body: &str) -> Self {
        Self::Answer {
            status,
            headers: headers
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
            body: body.as_bytes().to_vec(),
        }
    }
}

impl Default for Reply {
    fn default() -> Self {
        Self::status(200, "{}")
    }
}

// A scripted queue: each request takes the next answer, and an exhausted queue answers an empty 200
#[derive(Clone, Default)]
pub struct Replies(pub Arc<Mutex<VecDeque<Reply>>>);

impl Replies {
    fn take(&self) -> Reply {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_default()
    }
}

// Stands in for every handler kind a leaf can be handed and records the request it was given
pub struct FixtureClient {
    pub replies: Replies,
    pub seen: Seen,
}

impl FixtureClient {
    // The embedding leaf's signed POST: its reply echoes the URL the request was built for
    fn embedding_exchange(
        &self,
        request: EmbeddingRequest,
    ) -> Result<EmbeddingReply, EmbeddingTransferError> {
        let EmbeddingRequest { url, headers, body } = request;
        self.seen.push(Recorded {
            method: String::from("POST"),
            url: url.clone(),
            headers,
            body,
            deadline: None,
        });
        match self.replies.take() {
            Reply::Answer {
                status,
                headers: reply_headers,
                body: reply_body,
            } => Ok(EmbeddingReply {
                status,
                headers: reply_headers,
                body: reply_body,
                request: ReplyRequest {
                    method: String::from("POST"),
                    url,
                },
            }),
            Reply::Timeout => Err(EmbeddingTransferError::Timeout {
                elapsed: Duration::from_millis(125),
            }),
            Reply::Failed => Err(EmbeddingTransferError::Failed(TransportError::Closed)),
        }
    }

    // The async-invoke status lookup is a GET with no body at all
    fn status_exchange(
        &self,
        request: StatusWireRequest,
    ) -> Result<StatusReply, EmbeddingTransferError> {
        self.seen.push(Recorded {
            method: String::from("GET"),
            url: request.url,
            headers: request.headers,
            body: Vec::new(),
            deadline: None,
        });
        match self.replies.take() {
            Reply::Answer { status, body, .. } => Ok(StatusReply { status, body }),
            Reply::Timeout => Err(EmbeddingTransferError::Timeout {
                elapsed: Duration::from_millis(125),
            }),
            Reply::Failed => Err(EmbeddingTransferError::Failed(TransportError::Closed)),
        }
    }

    fn upstream(&self) -> Result<UpstreamResponse, TransportError> {
        match self.replies.take() {
            Reply::Answer {
                status,
                headers,
                body,
            } => Ok(UpstreamResponse {
                status,
                headers: header_map(&headers),
                body,
            }),
            Reply::Timeout | Reply::Failed => Err(TransportError::Closed),
        }
    }

    fn outbound(&self, request: OutboundRequest) -> Result<UpstreamResponse, TransportError> {
        self.seen.push(recorded(request));
        self.upstream()
    }

    fn rerank(&self, request: &RerankHttpRequest) -> Result<RerankHttpReply, RerankTransferError> {
        self.seen.push(Recorded {
            method: String::from("POST"),
            url: request.url.clone(),
            headers: request.headers.clone(),
            body: request.body.clone(),
            deadline: request.timeout,
        });
        match self.replies.take() {
            Reply::Answer {
                status,
                headers,
                body,
            } => Ok(RerankHttpReply {
                status,
                headers,
                body,
            }),
            Reply::Timeout => Err(RerankTransferError::Timeout),
            Reply::Failed => Err(RerankTransferError::Failed(TransportError::Closed)),
        }
    }

    // The image leaf's POST: a timeout and a closed transport differ, so the image error names both
    fn image_exchange(&self, request: ImageRequest) -> Result<ImageReply, ImageTransferError> {
        self.seen.push(Recorded {
            method: String::from("POST"),
            url: request.url,
            headers: request.headers,
            body: request.body,
            deadline: None,
        });
        match self.replies.take() {
            Reply::Answer {
                status,
                headers,
                body,
            } => Ok(ImageReply {
                status,
                headers,
                body,
            }),
            Reply::Timeout => Err(ImageTransferError::Timeout),
            Reply::Failed => Err(ImageTransferError::Failed(TransportError::Closed)),
        }
    }
}

impl EmbeddingTransport for FixtureClient {
    fn post(
        &self,
        request: EmbeddingRequest,
    ) -> impl Future<Output = Result<EmbeddingReply, EmbeddingTransferError>> + Send {
        ready(self.embedding_exchange(request))
    }
}

impl LegacyEmbeddingTransport for FixtureClient {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        ready(self.outbound(request))
    }
}

impl StatusTransport for FixtureClient {
    fn get(
        &self,
        request: StatusWireRequest,
    ) -> impl Future<Output = Result<StatusReply, EmbeddingTransferError>> + Send {
        ready(self.status_exchange(request))
    }
}

impl RerankTransport for FixtureClient {
    fn post(
        &self,
        request: RerankHttpRequest,
    ) -> impl Future<Output = Result<RerankHttpReply, RerankTransferError>> + Send {
        ready(self.rerank(&request))
    }

    fn post_with_logging(
        &self,
        request: RerankHttpRequest,
        _logging: &LoggingState,
    ) -> impl Future<Output = Result<RerankHttpReply, RerankTransferError>> + Send {
        ready(self.rerank(&request))
    }
}

impl ImageTransport for FixtureClient {
    fn post(
        &self,
        request: ImageRequest,
    ) -> impl Future<Output = Result<ImageReply, ImageTransferError>> + Send {
        ready(self.image_exchange(request))
    }
}

impl CountTokensTransport for FixtureClient {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        ready(self.outbound(request))
    }
}

impl TranscriptionTransport for FixtureClient {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        ready(self.outbound(request))
    }
}
