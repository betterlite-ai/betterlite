use std::pin::Pin;
use std::task::{Context, Poll};

use futures_util::stream::BoxStream;
use futures_util::{Stream, StreamExt as _};
use http::StatusCode;
use http::header::HeaderMap;

use crate::error::HttpError;

pub const PASSTHROUGH_UPSTREAM_ERROR_BODY_MAX_LOG_CHARS: usize = 4096; // ast-grep-ignore: no-long-names
const BODY_FRAMING_HEADERS: [&str; 2] = ["content-encoding", "content-length"];
const REPLACEMENT: char = '\u{FFFD}';

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
            return Poll::Ready(Some(Ok(std::mem::take(&mut this.prefix))));
        }
        match this.rest.as_mut() {
            Some(rest) => rest.poll_next_unpin(cx),
            None => Poll::Ready(None),
        }
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
                return (collected, futures_util::stream::empty().boxed());
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
    let preview = decode_utf8(&prefix);
    RelayedErrorBody {
        preview,
        status,
        headers: headers_without_body_framing(headers),
        stream: PrefixReplayStream::new(prefix, rest),
    }
}

// bytes.decode("utf-8", errors="replace"): an invalid sequence becomes one replacement character
fn decode_utf8(mut bytes: &[u8]) -> String {
    let mut decoded = String::with_capacity(bytes.len());
    loop {
        match std::str::from_utf8(bytes) {
            Ok(text) => {
                decoded.push_str(text);
                return decoded;
            }
            Err(error) => {
                let valid = error.valid_up_to();
                if let Some(text) = bytes
                    .get(..valid)
                    .and_then(|prefix| std::str::from_utf8(prefix).ok())
                {
                    decoded.push_str(text);
                }
                decoded.push(REPLACEMENT);
                let Some(invalid) = error.error_len() else {
                    return decoded;
                };
                bytes = bytes
                    .get(valid.saturating_add(invalid)..)
                    .unwrap_or_default();
            }
        }
    }
}

mod tests;
