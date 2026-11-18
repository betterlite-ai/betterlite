use std::future::{Future, ready};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_state::LoggingState;
use futures_util::{Stream, StreamExt as _};
use http::HeaderMap;
use secrecy::SecretString;
use std::pin::Pin;
use std::task::{Context, Poll};

use super::bedrock::agentcore_stream::transport::{
    HandlerKind, PostRequest, StreamHandler, StreamHead,
};
use super::bedrock::aws_settings::sources::Credentials;
use super::bedrock::converse_call::transport::{
    ConverseClientFactory, ConverseSigner as _, StreamClient,
};
use super::bedrock::credentials::error::CredentialError;
use super::bedrock::rerank::plan::CredentialSource;
use super::bedrock::rerank::transport::{
    ClientKind, RerankHttpReply, RerankHttpRequest, RerankTransport, TransferError,
};
use super::input::{Failure, Input, Observation};

#[derive(Clone)]
pub(super) struct Transport {
    pub(crate) input: Input,
    pub(crate) seen: Arc<Mutex<Observation>>,
    pub(crate) reads: Arc<AtomicUsize>,
}

impl RerankTransport for Transport {
    fn post(
        &self,
        request: RerankHttpRequest,
    ) -> impl Future<Output = Result<RerankHttpReply, TransferError>> + Send {
        let mut seen = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
        seen.sends = seen.sends.saturating_add(1);
        seen.url = request.url;
        seen.body = String::from_utf8(request.body).unwrap_or_default();
        seen.headers = request.headers.into_iter().collect();
        match self.input.native.failure {
            Failure::Timeout => return ready(Err(TransferError::Timeout)),
            Failure::Closed => return ready(Err(TransferError::Failed(TransportError::Closed))),
            Failure::None => {}
        }
        ready(Ok(RerankHttpReply {
            status: self.input.status,
            headers: vec![("x-request-id".to_owned(), "request-1".to_owned())],
            body: self.input.body.as_bytes().to_vec(),
        }))
    }

    fn post_with_logging(
        &self,
        request: RerankHttpRequest,
        _logging: &LoggingState,
    ) -> impl Future<Output = Result<RerankHttpReply, TransferError>> + Send {
        self.post(request)
    }
}

#[async_trait]
impl StreamHandler for Transport {
    fn kind(&self) -> HandlerKind {
        HandlerKind::Async
    }

    async fn post(&self, request: PostRequest) -> Result<StreamHead, TransportError> {
        match self.input.native.failure {
            Failure::Timeout => {
                return Err(TransportError::Rejected(
                    betterlite_http::error::HttpError::Timeout,
                ));
            }
            Failure::Closed => return Err(TransportError::Closed),
            Failure::None => {}
        }
        {
            let mut seen = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
            seen.sends = seen.sends.saturating_add(1);
            seen.url = request.url.to_string();
            seen.body = String::from_utf8(request.body).unwrap_or_default();
            seen.headers = request
                .headers
                .iter()
                .map(|(name, value)| {
                    (
                        name.to_string(),
                        value.to_str().unwrap_or_default().to_owned(),
                    )
                })
                .collect();
        }
        let reads = Arc::clone(&self.reads);
        let bytes: Vec<Vec<u8>> = if request.stream && self.input.status == 200 {
            self.input.frames.clone()
        } else {
            vec![self.input.body.as_bytes().to_vec()]
        };
        let chunks = ReadStream {
            bytes: bytes.into_iter(),
            reads,
            seen: Arc::clone(&self.seen),
        }
        .boxed();
        Ok(StreamHead {
            status: self.input.status,
            headers: HeaderMap::from_iter([(
                http::HeaderName::from_static("x-request-id"),
                http::HeaderValue::from_static("request-1"),
            )]),
            chunks,
        })
    }
}

impl ConverseClientFactory for Transport {
    type Buffered = Self;

    fn buffered(&self, _kind: ClientKind, timeout: Option<Duration>) -> Self {
        let mut seen = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
        seen.factory = seen.factory.saturating_add(1);
        seen.factory_timeout_ms = timeout.map(|value| value.as_millis());
        self.clone()
    }

    fn stream(&self, _kind: ClientKind, timeout: Option<Duration>) -> StreamClient {
        let mut seen = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
        seen.factory = seen.factory.saturating_add(1);
        seen.factory_timeout_ms = timeout.map(|value| value.as_millis());
        StreamClient {
            handler: Arc::new(self.clone()),
            timeout: timeout.unwrap_or(Duration::from_secs(10)),
        }
    }
}

pub(super) struct Source(pub(super) AtomicUsize);

impl CredentialSource for Source {
    fn resolve(
        &self,
        settings: AwsSettings,
        _region: String,
    ) -> impl Future<Output = Result<Credentials, CredentialError>> + Send {
        self.0.fetch_add(1, Ordering::Relaxed);
        ready(Ok(Credentials {
            access_key_id: settings
                .access_key_id
                .unwrap_or_else(|| SecretString::from("fixture-access")),
            secret_access_key: settings
                .secret_access_key
                .unwrap_or_else(|| SecretString::from("fixture-secret")),
            session_token: settings.session_token,
        }))
    }
}

struct ReadStream {
    bytes: std::vec::IntoIter<Vec<u8>>,
    reads: Arc<AtomicUsize>,
    seen: Arc<Mutex<Observation>>,
}

impl Stream for ReadStream {
    type Item = Result<Vec<u8>, betterlite_http::error::HttpError>;
    fn poll_next(mut self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Ready(self.bytes.next().map(|bytes| {
            self.reads.fetch_add(1, Ordering::Relaxed);
            Ok(bytes)
        }))
    }
}

impl Drop for ReadStream {
    fn drop(&mut self) {
        let mut seen = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
        seen.drops = seen.drops.saturating_add(1);
    }
}

pub(super) struct Signer<'seen>(pub(super) &'seen Transport);
impl super::bedrock::converse_call::transport::ConverseSigner for Signer<'_> {
    fn sign(
        &self,
        request: super::bedrock::signing::endpoint::RequestHeaders<'_>,
    ) -> Result<
        super::bedrock::signing::request::AwsSignedRequest,
        super::bedrock::signing::error::SigningError,
    > {
        self.0
            .seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .signed_body = request.data.bytes().to_vec();
        super::bedrock::converse_call::transport::SigV4Signer.sign(request)
    }
}
