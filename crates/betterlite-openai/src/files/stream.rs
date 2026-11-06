use std::mem;
use std::num::NonZeroUsize;
use std::pin::Pin;
use std::str;
use std::sync::{Arc, OnceLock};
use std::task::{Context, Poll};

use betterlite_http::error::HttpError;
use futures_util::stream::BoxStream;
use futures_util::{Stream, StreamExt as _};
use http::HeaderMap;
use indexmap::IndexMap;

use crate::files::error::FilesError;

// httpx's iter_bytes chunk size when the caller names none
pub const DEFAULT_CHUNK_SIZE: usize = 1024 * 1024;

// an upstream error body is read only this far before the status error is built
const ERROR_BODY_LIMIT: usize = 64 * 1024;

// How the SDK's response context was left: the three-None exit, an exit carrying the error, or an early close
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamExit {
    Completed,
    Failed(String),
    Closed,
}

// LiteLLM's _stream generator: re-chunks like httpx iter_bytes and leaves the response context exactly once
pub struct FileContentStream {
    inner: BoxStream<'static, Result<Vec<u8>, HttpError>>,
    buffer: Vec<u8>,
    chunk_size: usize,
    upstream_done: bool,
    exit: Arc<OnceLock<StreamExit>>,
}

pub struct FileContentStreamingResult {
    pub stream: FileContentStream,
    pub headers: IndexMap<String, String>,
    pub exit: Arc<OnceLock<StreamExit>>,
}

impl FileContentStreamingResult {
    #[must_use]
    pub fn new(
        body: BoxStream<'static, Result<Vec<u8>, HttpError>>,
        headers: &HeaderMap,
        chunk_size: Option<NonZeroUsize>,
    ) -> Self {
        let exit = Arc::new(OnceLock::new());
        Self {
            stream: FileContentStream {
                inner: body,
                buffer: Vec::new(),
                chunk_size: chunk_size.map_or(DEFAULT_CHUNK_SIZE, NonZeroUsize::get),
                upstream_done: false,
                exit: Arc::clone(&exit),
            },
            headers: flatten_headers(headers),
            exit,
        }
    }
}

// httpx's dict(response.headers): lower-case names, repeated names joined with ", ", bytes read as UTF-8 then Latin-1
#[must_use]
pub fn flatten_headers(headers: &HeaderMap) -> IndexMap<String, String> {
    let mut flat: IndexMap<String, String> = IndexMap::new();
    for (header_name, header_value) in headers {
        let raw = header_value.as_bytes();
        let text = str::from_utf8(raw).map_or_else(
            |_| raw.iter().map(|byte| char::from(*byte)).collect(),
            str::to_owned,
        );
        flat.entry(header_name.as_str().to_owned())
            .and_modify(|joined| {
                joined.push_str(", ");
                joined.push_str(&text);
            })
            .or_insert(text);
    }
    flat
}

impl Stream for FileContentStream {
    type Item = Result<Vec<u8>, FilesError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            if this.buffer.len() >= this.chunk_size {
                let rest = this.buffer.split_off(this.chunk_size);
                return Poll::Ready(Some(Ok(mem::replace(&mut this.buffer, rest))));
            }
            if this.upstream_done && !this.buffer.is_empty() {
                return Poll::Ready(Some(Ok(mem::take(&mut this.buffer))));
            }
            if this.upstream_done {
                this.exit.get_or_init(|| StreamExit::Completed);
                return Poll::Ready(None);
            }
            match this.inner.poll_next_unpin(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Some(Ok(bytes))) => this.buffer.extend(bytes),
                Poll::Ready(None) => this.upstream_done = true,
                Poll::Ready(Some(Err(source))) => {
                    this.exit
                        .get_or_init(|| StreamExit::Failed(source.to_string()));
                    this.buffer.clear();
                    this.upstream_done = true;
                    return Poll::Ready(Some(Err(FilesError::Body { source })));
                }
            }
        }
    }
}

// Python never exits the context of a generator that was never started; dropping always releases the response
impl Drop for FileContentStream {
    fn drop(&mut self) {
        self.exit.get_or_init(|| StreamExit::Closed);
    }
}

pub async fn read_error_body(mut body: BoxStream<'static, Result<Vec<u8>, HttpError>>) -> Vec<u8> {
    let mut collected = Vec::new();
    while collected.len() < ERROR_BODY_LIMIT
        && let Some(Ok(chunk)) = body.next().await
    {
        collected.extend(chunk);
    }
    collected
}
