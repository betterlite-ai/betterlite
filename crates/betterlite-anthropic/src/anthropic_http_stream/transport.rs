use std::error::Error as StdError;
use std::future::Future;
use std::time::Duration;

use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::StreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::aleph_alpha::lines::decode_lines;
use futures_util::StreamExt as _;
use futures_util::stream::BoxStream;
use http::{HeaderMap, Method};
use tokio::time::timeout;
use url::Url;

use crate::stream_decode::TransportChunk;
use crate::stream_decode::error::ReceiveError;

use super::error::{PostFailure, StatusFailure, StreamFailure};

pub type LineChunks<Failure> = BoxStream<'static, Result<TransportChunk, ReceiveError<Failure>>>;

#[derive(Debug)]
pub struct StreamPost {
    pub url: Url,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
    pub stream: bool,
    pub deadline: Duration,
}

pub struct StreamReply<Failure> {
    pub status_code: u16,
    pub headers: HeaderMap,
    pub lines: LineChunks<Failure>,
}

pub trait StreamTransport: Send + Sync {
    type Failure: StdError + Send + Sync + 'static;

    fn post(
        &self,
        post: StreamPost,
    ) -> impl Future<Output = Result<StreamReply<Self::Failure>, PostFailure<Self::Failure>>> + Send;
}

// http_handler._STREAMING_ERROR_BODY_READ_TIMEOUT_SECONDS, the cap LiteLLM puts on reading a streaming error body
const ERROR_BODY_READ: Duration = Duration::from_secs(5);

impl StreamTransport for ProxiedClient {
    type Failure = StreamFailure;

    async fn post(
        &self,
        post: StreamPost,
    ) -> Result<StreamReply<StreamFailure>, PostFailure<StreamFailure>> {
        let response = self
            .send_following(OutboundRequest {
                method: Method::POST,
                url: post.url,
                query: Vec::new(),
                headers: post.headers,
                body: RequestBody::Json(post.body),
                deadline: post.deadline,
            })
            .await
            .map_err(post_failure)?;
        let status_code = response.response.status().as_u16();
        let headers = response.response.headers().clone();
        if !response.response.status().is_success() {
            return Err(PostFailure::Status(Box::new(
                error_status(response, self.limits.body_bytes).await,
            )));
        }
        let lines = decode_lines(response, self.limits.body_bytes)
            .map(|line| match line {
                Ok(text) => Ok(TransportChunk::Text(text)),
                Err(source) => Err(ReceiveError::Other {
                    source: StreamFailure::Lines(source),
                }),
            })
            .boxed();
        Ok(StreamReply {
            status_code,
            headers,
            lines,
        })
    }
}

// LiteLLM reads an error body under a cap and keeps the upstream status with an empty message when the read fails
async fn error_status(response: StreamResponse, limit: usize) -> StatusFailure {
    let status_code = response.response.status().as_u16();
    let response_headers = Some(response.response.headers().clone());
    let body = match timeout(ERROR_BODY_READ, response.buffered(limit)).await {
        Ok(Ok(reply)) => reply.body,
        Ok(Err(_)) | Err(_) => Vec::new(),
    };
    StatusFailure {
        status_code,
        body,
        exception_headers: None,
        response_headers,
    }
}

// The handler turns a timeout into litellm.Timeout, which make_call re-raises; any other send error becomes a 500
const fn post_failure(source: TransportError) -> PostFailure<StreamFailure> {
    let timed = matches!(
        source,
        TransportError::Deadline(_) | TransportError::ConnectTimeout(_)
    );
    let failure = StreamFailure::Transport(source);
    if timed {
        PostFailure::Known(failure)
    } else {
        PostFailure::Unknown(failure)
    }
}
