use std::future::Future;
use std::future::ready;
use std::io::Error as IoError;

use serde_json::value::RawValue;
use tokio::sync::mpsc;

use super::support::{guard_at, quiet, request};
use crate::cato::error::ModelStreamError;
use crate::presidio_response::wire::ChatModelResponse;
use crate::presidio_streams::StreamEvent;
use crate::presidio_streams::assembly::StreamAssembly;

struct NoAssembly;

impl StreamAssembly for NoAssembly {
    type Error = IoError;

    fn is_anthropic_sse_stream(&self, _chunk: &[u8]) -> bool {
        false
    }

    fn stream_chunk_builder(
        &self,
        _chunks: &[Box<RawValue>],
        _messages: Option<&RawValue>,
    ) -> impl Future<Output = Result<Option<ChatModelResponse>, Self::Error>> + Send {
        ready(Ok(None))
    }

    fn convert_to_streaming(
        &self,
        _response: &ChatModelResponse,
    ) -> Result<Box<RawValue>, Self::Error> {
        Err(IoError::other("the fake assembly does not convert"))
    }

    fn assemble_anthropic_sse_stream(
        &self,
        _chunks: &[Vec<u8>],
    ) -> impl Future<Output = Option<ChatModelResponse>> + Send {
        ready(None)
    }

    fn model_response_text(&self, _response: &ChatModelResponse) -> String {
        String::new()
    }

    fn anthropic_sse_chunks(
        &self,
        _response: &ChatModelResponse,
    ) -> Result<Vec<Vec<u8>>, Self::Error> {
        Err(IoError::other("the fake assembly does not chunk"))
    }
}

pub(super) async fn streaming_iterator_relays_events() {
    let server = quiet();
    let guard = guard_at(&server);
    let mut data = request(r#"{"model":"gpt-4o"}"#);
    let event = RawValue::from_string(r#"{"object":"response"}"#.to_owned()).expect("the event");
    let (input, mut input_rx) = mpsc::channel::<Result<StreamEvent, ModelStreamError>>(4);
    let (out, mut out_rx) = mpsc::channel::<StreamEvent>(4);
    input
        .send(Ok(StreamEvent::Other(event)))
        .await
        .expect("the input event");
    drop(input);
    guard
        .streaming_iterator(&mut data, &NoAssembly, &mut input_rx, &out)
        .await
        .expect("the stream is relayed");
    drop(out);
    assert!(matches!(out_rx.recv().await, Some(StreamEvent::Other(_))));
    assert!(out_rx.recv().await.is_none());
}
