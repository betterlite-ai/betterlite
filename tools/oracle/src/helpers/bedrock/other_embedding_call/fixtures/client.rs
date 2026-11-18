use std::collections::VecDeque;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_bedrock::bedrock_embedding_io::transport::{
    ClientFactory, EmbeddingReply, EmbeddingRequest, EmbeddingTransport, ReplyRequest, Timeout,
    TransferError,
};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use serde_json::{Value, json};

use super::super::input::{Failure, Reply};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Sent {
    pub(crate) url: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
}

#[derive(Clone, Default)]
pub(crate) struct Seen(Arc<Mutex<Vec<Sent>>>);

impl Seen {
    pub(crate) fn requests(&self) -> Vec<Sent> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn push(&self, request: Sent) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
    }
}

#[derive(Clone, Default)]
pub(crate) struct Replies(Arc<Mutex<VecDeque<Reply>>>);

impl Replies {
    fn queue(replies: &[Reply]) -> Self {
        Self(Arc::new(Mutex::new(replies.iter().cloned().collect())))
    }

    fn take(&self) -> Option<Reply> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
    }
}

// the fixture client stands in for the HTTPHandler, the AsyncHTTPHandler and the proxied client
pub(crate) struct FixtureClient {
    replies: Replies,
    seen: Seen,
}

impl FixtureClient {
    fn new(replies: Replies) -> Self {
        Self {
            replies,
            seen: Seen::default(),
        }
    }

    pub(crate) fn requests(&self) -> Vec<Sent> {
        self.seen.requests()
    }

    fn answer(&self, url: &str) -> Result<EmbeddingReply, TransferError> {
        match self.replies.take() {
            None => Err(TransferError::Failed(TransportError::Closed)),
            Some(Reply {
                failure: Some(Failure::Timeout),
                ..
            }) => Err(TransferError::Timeout {
                elapsed: Duration::from_millis(125),
            }),
            Some(Reply {
                failure: Some(Failure::Failed),
                ..
            }) => Err(TransferError::Failed(TransportError::Closed)),
            Some(reply) => Ok(EmbeddingReply {
                status: reply.status,
                headers: reply.headers(),
                body: reply.body.into_bytes(),
                request: ReplyRequest {
                    method: String::from("POST"),
                    url: url.to_owned(),
                },
            }),
        }
    }

    fn answer_upstream(&self) -> Result<UpstreamResponse, TransportError> {
        match self.replies.take() {
            None
            | Some(Reply {
                failure: Some(Failure::Timeout | Failure::Failed),
                ..
            }) => Err(TransportError::Closed),
            Some(reply) => Ok(UpstreamResponse {
                status: reply.status,
                headers: reply
                    .headers
                    .iter()
                    .filter_map(|(name, value)| {
                        Some((
                            http::HeaderName::try_from(name.as_str()).ok()?,
                            http::HeaderValue::try_from(value.as_str()).ok()?,
                        ))
                    })
                    .collect(),
                body: reply.body.into_bytes(),
            }),
        }
    }
}

impl EmbeddingTransport for FixtureClient {
    fn post(
        &self,
        request: EmbeddingRequest,
    ) -> impl Future<Output = Result<EmbeddingReply, TransferError>> + Send {
        self.seen.push(Sent {
            url: request.url.clone(),
            headers: request.headers,
            body: request.body,
        });
        ready(self.answer(&request.url))
    }
}

impl betterlite_openai::cohere_legacy_embedding::execute::LegacyEmbeddingTransport
    for FixtureClient
{
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        let body = match request.body {
            RequestBody::Json(bytes) | RequestBody::Bytes { data: bytes, .. } => bytes,
            RequestBody::Empty | RequestBody::Multipart { .. } => Vec::new(),
        };
        self.seen.push(Sent {
            url: request.url.to_string(),
            headers: request
                .headers
                .iter()
                .map(|(name, value)| {
                    (
                        name.as_str().to_owned(),
                        value.to_str().unwrap_or_default().to_owned(),
                    )
                })
                .collect(),
            body,
        });
        ready(self.answer_upstream())
    }
}

pub(crate) struct Factory {
    replies: Replies,
    seen: Seen,
    calls: Mutex<Vec<Value>>,
}

impl Factory {
    pub(crate) fn new(replies: &[Reply]) -> Self {
        Self {
            replies: Replies::queue(replies),
            seen: Seen::default(),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn replies(&self) -> Replies {
        self.replies.clone()
    }

    pub(crate) fn requests(&self) -> Vec<Sent> {
        self.seen.requests()
    }

    pub(crate) fn calls(&self) -> Vec<Value> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, kind: &str, provider: Option<&str>, timeout: Option<&Timeout>) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({
                "kind": kind,
                "provider": provider,
                "timeout": timeout.map(Timeout::fields),
            }));
    }

    fn client(&self) -> FixtureClient {
        FixtureClient {
            replies: self.replies.clone(),
            seen: self.seen.clone(),
        }
    }
}

impl ClientFactory for Factory {
    type Client = FixtureClient;

    fn sync_client(&self, timeout: Option<Timeout>) -> Self::Client {
        self.record("sync", None, timeout.as_ref());
        self.client()
    }

    fn async_client(&self, provider: &str, timeout: Option<Timeout>) -> Self::Client {
        self.record("async", Some(provider), timeout.as_ref());
        self.client()
    }
}

pub(crate) fn supplied_client(factory: &Factory) -> FixtureClient {
    FixtureClient::new(factory.replies())
}
