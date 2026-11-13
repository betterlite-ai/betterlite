use std::convert::Infallible;
use std::error::Error as StdError;
use std::str::Utf8Error;

use thiserror::Error;

use crate::stream_events::error::StreamEventError;

// Python tells a transport ValueError apart from every other exception, and only the first is wrapped.
#[derive(Debug, Clone, Error)]
pub enum ReceiveError<SourceFailure> {
    #[error("{source}")]
    Value { source: SourceFailure },
    #[error("{source}")]
    Other { source: SourceFailure },
}

// The counter failure only reaches this enum inside StreamEventError, so the bound sits on the parameter.
pub trait CountFailure: StdError + Send + Sync + 'static {}

impl<Failure> CountFailure for Failure where Failure: StdError + Send + Sync + 'static {}

#[derive(Debug, Error)]
pub enum DecodeError<SourceFailure, CounterFailure: CountFailure = Infallible> {
    // RuntimeError from the receive branch of ModelResponseIterator.__next__ and __anext__.
    #[error("Error receiving chunk from stream: {source}")]
    Receiving { source: SourceFailure },
    // Every non-ValueError from the source keeps its own class.
    #[error("{source}")]
    Other { source: SourceFailure },
    // RuntimeError from the parse branch, with the chunk LiteLLM puts in its message.
    #[error("Error parsing chunk: {source},\nReceived chunk: {chunk:?}")]
    Parsing { source: Utf8Error, chunk: Vec<u8> },
    // Errors chunk_parser raises keep their class, including the native AnthropicError.
    #[error("{0}")]
    Event(#[source] StreamEventError<CounterFailure>),
}
