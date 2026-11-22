use std::mem::take;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures_util::stream::{BoxStream, empty};
use futures_util::{Stream, StreamExt as _};
use http::StatusCode;
use http::header::HeaderMap;

use crate::error::HttpError;

pub const PASSTHROUGH_UPSTREAM_ERROR_BODY_MAX_LOG_CHARS: usize = 4096; // ast-grep-ignore: no-long-names
const BODY_FRAMING_HEADERS: [&str; 2] = ["content-encoding", "content-length"];

// the upstream body as it arrives; a stream item's error is the transport raising mid-read
pub type ChunkStream = BoxStream<'static, Result<Vec<u8>, HttpError>>;

// _PrefixReplayStream: the consumed prefix, then the rest of the upstream body without buffering it
pub struct PrefixReplayStream {
    prefix: Vec<u8>,
    rest: Option<ChunkStream>,
}

impl PrefixReplayStream {
    #[must_use]
    pub fn new(prefix: Vec<u8>, rest: ChunkStream) -> Self {
        Self {
            prefix,
            rest: Some(rest),
        }
    }

    // aclose(): dropping the rest closes the upstream response, the way httpx releases its connection
    pub fn aclose(&mut self) {
        if let Some(rest) = self.rest.take() {
            drop(rest);
        }
    }
}

impl Stream for PrefixReplayStream {
    type Item = Result<Vec<u8>, HttpError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if !this.prefix.is_empty() {
            return Poll::Ready(Some(Ok(take(&mut this.prefix))));
        }
        let Some(rest) = this.rest.as_mut() else {
            return Poll::Ready(None);
        };
        rest.poll_next_unpin(cx)
    }
}

// _read_error_body_preview: collect until the byte budget is passed, then hand the unread stream back
pub async fn read_error_body_preview(mut stream: ChunkStream) -> (Vec<u8>, ChunkStream) {
    let mut collected = Vec::new();
    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(bytes) => {
                collected.extend_from_slice(&bytes);
                // ast-grep-ignore: no-long-names
                if collected.len() > PASSTHROUGH_UPSTREAM_ERROR_BODY_MAX_LOG_CHARS {
                    break;
                }
            }
            Err(error) => {
                tracing::warn!(
                    "pass_through_endpoint: upstream error body read failed after {} bytes: {error}",
                    collected.len()
                );
                return (collected, empty().boxed());
            }
        }
    }
    (collected, stream)
}

// _headers_without_body_framing: the body is replayed decoded, so its framing no longer describes it
#[must_use]
pub fn headers_without_body_framing(headers: &HeaderMap) -> HeaderMap {
    let mut kept = HeaderMap::new();
    for (name, value) in headers {
        if !BODY_FRAMING_HEADERS.contains(&name.as_str()) {
            kept.append(name, value.clone());
        }
    }
    kept
}

pub struct RelayedErrorBody {
    pub preview: String,
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub stream: PrefixReplayStream,
}

// _error_body_preview_and_relay: a bounded text preview plus a response that replays every upstream byte
pub async fn error_body_preview_and_relay(
    status: StatusCode,
    headers: &HeaderMap,
    body: ChunkStream,
) -> RelayedErrorBody {
    let (prefix, rest) = read_error_body_preview(body).await;
    // bytes.decode("utf-8", errors="replace"): an invalid sequence becomes one replacement character
    #[expect(
        clippy::disallowed_methods,
        reason = "LiteLLM decodes the preview with errors=replace"
    )]
    let preview = String::from_utf8_lossy(&prefix).into_owned();
    RelayedErrorBody {
        preview,
        status,
        headers: headers_without_body_framing(headers),
        stream: PrefixReplayStream::new(prefix, rest),
    }
}

mod tests;
