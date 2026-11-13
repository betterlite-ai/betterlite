use super::chunk::StreamChunk;
use super::error::StreamError;
use super::{GeminiStreamDecoder, StreamOptions};
use http::header::HeaderMap;
use std::future::Future;
use std::pin::Pin;
use uuid::Uuid;
pub type StreamFuture<'source, Output> = Pin<Box<dyn Future<Output = Output> + Send + 'source>>;
pub trait FragmentReader {
    fn next_fragment(&mut self) -> Result<Option<String>, StreamError>;
    fn close(&mut self) -> Result<(), StreamError>;
}
pub trait ResponseClose {
    fn close_response(&mut self) -> Result<(), StreamError>;
}
pub trait AsyncFragmentReader {
    fn next_fragment(&mut self) -> StreamFuture<'_, Result<Option<String>, StreamError>>;
    fn close(&mut self) -> StreamFuture<'_, Result<(), StreamError>>;
}
// Again means the fragment carried no chunk, which LiteLLM's iterator reports as a none item
enum Advance {
    Again,
    Chunk(Box<StreamChunk>),
    Failed(Box<StreamError>),
    Done,
}
pub struct GeminiStream<Reader, Response> {
    decoder: GeminiStreamDecoder,
    finished: bool,
    reader: Reader,
    response: Option<Response>,
}
impl<Reader, Response> GeminiStream<Reader, Response> {
    pub fn new(
        reader: Reader,
        response: Option<Response>,
        options: StreamOptions,
        headers: &HeaderMap,
        ids: Box<dyn FnMut() -> Uuid + Send>,
    ) -> Self {
        Self {
            decoder: GeminiStreamDecoder::new(options, headers, ids),
            finished: false,
            reader,
            response,
        }
    }
    fn advance(&mut self, fragment: Result<Option<String>, StreamError>) -> Advance {
        if self.finished {
            return Advance::Done;
        }
        match fragment {
            Ok(Some(fragment)) => match self.decoder.push(fragment.as_bytes()) {
                Ok(None) => Advance::Again,
                Ok(Some(chunk)) => Advance::Chunk(Box::new(chunk)),
                Err(error) => self.failed(error),
            },
            Ok(None) => match self.decoder.finish() {
                Ok(None) => Advance::Done,
                Ok(Some(chunk)) => Advance::Chunk(Box::new(chunk)),
                Err(error) => self.failed(error),
            },
            Err(error) => self.failed(error),
        }
    }
    fn failed(&mut self, error: StreamError) -> Advance {
        self.finished = true;
        Advance::Failed(Box::new(error))
    }
}
impl<Reader: FragmentReader, Response: ResponseClose> GeminiStream<Reader, Response> {
    // LiteLLM logs a failed close and keeps going, so the response still closes when the reader fails
    pub fn close(&mut self) {
        report_close(self.reader.close(), "reader");
        if let Some(response) = self.response.as_mut() {
            report_close(response.close_response(), "response");
        }
    }
}
impl<Reader: FragmentReader, Response: ResponseClose> Iterator for GeminiStream<Reader, Response> {
    type Item = Result<Option<StreamChunk>, StreamError>;
    fn next(&mut self) -> Option<Self::Item> {
        let fragment = self.reader.next_fragment();
        match self.advance(fragment) {
            Advance::Again => Some(Ok(None)),
            Advance::Chunk(chunk) => Some(Ok(Some(*chunk))),
            Advance::Failed(error) => Some(Err(*error)),
            Advance::Done => None,
        }
    }
}
impl<Reader: AsyncFragmentReader, Response: ResponseClose> GeminiStream<Reader, Response> {
    pub async fn next_chunk(&mut self) -> Option<Result<Option<StreamChunk>, StreamError>> {
        let fragment = self.reader.next_fragment().await;
        match self.advance(fragment) {
            Advance::Again => Some(Ok(None)),
            Advance::Chunk(chunk) => Some(Ok(Some(*chunk))),
            Advance::Failed(error) => Some(Err(*error)),
            Advance::Done => None,
        }
    }
    pub async fn aclose(&mut self) {
        report_close(self.reader.close().await, "reader");
        if let Some(response) = self.response.as_mut() {
            report_close(response.close_response(), "response");
        }
    }
}
fn report_close(result: Result<(), StreamError>, owner: &'static str) {
    if let Err(error) = result {
        tracing::debug!(error = %error, owner, "closing the Gemini stream owner failed");
    }
}
