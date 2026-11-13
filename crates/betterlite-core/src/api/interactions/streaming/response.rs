use std::collections::VecDeque;
use std::error::Error;

use async_trait::async_trait;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::StreamResponse;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_openai::aleph_alpha::lines::LineDecoder;
use futures_util::StreamExt as _;
use futures_util::stream::BoxStream;
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

pub struct StreamLineReader {
    body: BoxStream<'static, Result<Vec<u8>, HttpError>>,
    decoder: LineDecoder,
    pending: VecDeque<String>,
    finished: bool,
}

#[async_trait]
impl InteractionsLineStream for StreamLineReader {
    type Error = InteractionSourceError;

    async fn next_line(&mut self) -> Result<Option<String>, Self::Error> {
        loop {
            if let Some(line) = self.pending.pop_front() {
                return Ok(Some(line));
            }
            if self.finished {
                return Ok(None);
            }
            match self.body.next().await {
                Some(Ok(bytes)) => self
                    .decoder
                    .push(&bytes, &mut self.pending)
                    .map_err(InteractionSourceError::Line)?,
                Some(Err(source)) => return Err(InteractionSourceError::Body { source }),
                None => {
                    self.decoder.flush(&mut self.pending);
                    self.finished = true;
                }
            }
        }
    }
}

impl InteractionsAsyncResponse for StreamResponse {
    type Error = InteractionSourceError;
    type Lines = StreamLineReader;

    fn headers(&self) -> IndexMap<String, String> {
        header_index(self.response.headers())
    }

    fn aiter_lines(self) -> Self::Lines {
        StreamLineReader {
            body: self.body_stream(),
            decoder: LineDecoder::new(MAX_LINE_BYTES),
            pending: VecDeque::new(),
            finished: false,
        }
    }
}

pub struct InteractionBufferedResponse {
    pub headers: IndexMap<String, String>,
    pub body: Vec<u8>,
}

impl InteractionsSyncResponse for InteractionBufferedResponse {
    type Error = InteractionSourceError;
    type Lines = std::vec::IntoIter<Result<String, InteractionSourceError>>;

    fn headers(&self) -> IndexMap<String, String> {
        self.headers.clone()
    }

    fn iter_lines(self) -> Self::Lines {
        let mut decoder = LineDecoder::new(MAX_LINE_BYTES);
        let mut pending = VecDeque::new();
        let mut lines: Vec<Result<String, InteractionSourceError>> = Vec::new();
        if let Err(error) = decoder.push(&self.body, &mut pending) {
            lines.push(Err(InteractionSourceError::Line(error)));
        }
        decoder.flush(&mut pending);
        lines.extend(
            pending
                .into_iter()
                .map(Ok::<String, InteractionSourceError>),
        );
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
