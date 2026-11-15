use std::collections::VecDeque;
use std::error::Error;
use std::vec::IntoIter;

use async_trait::async_trait;
use betterlite_http::proxy::StreamResponse;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_openai::aleph_alpha::lines::{LineDecoder, LineStream, decode_lines};
use futures_util::StreamExt as _;
use indexmap::IndexMap;

use super::error::InteractionSourceError;

pub const MAX_LINE_BYTES: usize = TransportLimits::DEFAULT.body_bytes;

#[async_trait]
pub trait InteractionsLineStream {
    type Error;

    async fn next_line(&mut self) -> Result<Option<String>, Self::Error>;
}

pub trait InteractionsAsyncResponse {
    type Error: Error + Send + Sync + 'static;
    type Lines: InteractionsLineStream<Error = Self::Error>;

    fn headers(&self) -> IndexMap<String, String>;

    fn aiter_lines(self) -> Self::Lines;
}

pub trait InteractionsSyncResponse {
    type Error: Error + Send + Sync + 'static;
    type Lines: Iterator<Item = Result<String, Self::Error>>;

    fn headers(&self) -> IndexMap<String, String>;

    fn iter_lines(self) -> Self::Lines;
}

// httpx hands the response body on as lines, and decode_lines is that reader: a bounded line buffer
#[async_trait]
impl InteractionsLineStream for LineStream {
    type Error = InteractionSourceError;

    async fn next_line(&mut self) -> Result<Option<String>, Self::Error> {
        match self.next().await {
            Some(Ok(line)) => Ok(Some(line)),
            Some(Err(source)) => Err(InteractionSourceError::Line(source)),
            None => Ok(None),
        }
    }
}

impl InteractionsAsyncResponse for StreamResponse {
    type Error = InteractionSourceError;
    type Lines = LineStream;

    fn headers(&self) -> IndexMap<String, String> {
        header_index(self.response.headers())
    }

    fn aiter_lines(self) -> Self::Lines {
        decode_lines(self, MAX_LINE_BYTES)
    }
}

pub struct InteractionBufferedResponse {
    pub headers: IndexMap<String, String>,
    pub body: Vec<u8>,
}

impl InteractionsSyncResponse for InteractionBufferedResponse {
    type Error = InteractionSourceError;
    type Lines = IntoIter<Result<String, InteractionSourceError>>;

    fn headers(&self) -> IndexMap<String, String> {
        self.headers.clone()
    }

    fn iter_lines(self) -> Self::Lines {
        let mut decoder = LineDecoder::new(MAX_LINE_BYTES);
        let mut pending = VecDeque::new();
        let mut lines: Vec<Result<String, InteractionSourceError>> = Vec::new();
        let oversized = decoder.push(&self.body, &mut pending).err();
        if oversized.is_none() {
            decoder.flush(&mut pending);
        }
        lines.extend(
            pending
                .into_iter()
                .map(Ok::<String, InteractionSourceError>),
        );
        lines.extend(oversized.map(|error| Err(InteractionSourceError::Line(error))));
        lines.into_iter()
    }
}

fn header_index(headers: &http::HeaderMap) -> IndexMap<String, String> {
    let mut index = IndexMap::new();
    for (name, value) in headers {
        match value.to_str() {
            Ok(text) => {
                index.insert(name.as_str().to_owned(), text.to_owned());
            }
            Err(_) => {
                tracing::debug!(
                    header = name.as_str(),
                    "the response header value is not text"
                );
            }
        }
    }
    index
}
