use std::collections::VecDeque;
use std::future::pending;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_bedrock::realtime_events::stream::{
    BedrockBidirectionalStream, BedrockInputStream, BedrockOutputChunk, BedrockOutputStream,
    BedrockPayloadPart, StreamError,
};
use betterlite_models::decisions::Json;

use super::fakes::{FakeError, stream_error};
use super::input::Block;

#[derive(Clone)]
pub(super) struct StreamProfile {
    sent: Arc<Mutex<Vec<Json>>>,
    pub(super) closed: Arc<AtomicBool>,
}

impl StreamProfile {
    pub(super) fn sent(&self) -> Vec<Json> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

struct ScriptedInput {
    sent: Arc<Mutex<Vec<Json>>>,
    send_failure: Option<usize>,
    closed: Arc<AtomicBool>,
}

#[async_trait]
impl BedrockInputStream for ScriptedInput {
    type Error = FakeError;

    async fn send(&self, event: Json) -> Result<(), StreamError<FakeError>> {
        let index = {
            let mut sent = self.sent.lock().unwrap_or_else(PoisonError::into_inner);
            let index = sent.len();
            sent.push(event);
            index
        };
        if self.send_failure == Some(index) {
            return Err(stream_error("the input send failed", "ClientStreamError"));
        }
        Ok(())
    }

    async fn close(&self) -> Result<(), StreamError<FakeError>> {
        self.closed.store(true, Ordering::SeqCst);
        Ok(())
    }
}

struct ScriptedPart {
    bytes: Option<Vec<u8>>,
}

impl BedrockPayloadPart for ScriptedPart {
    fn bytes(&self) -> Option<&[u8]> {
        self.bytes.as_deref()
    }
}

struct ScriptedValue {
    part: ScriptedPart,
}

impl BedrockOutputChunk for ScriptedValue {
    fn value(&self) -> Option<&dyn BedrockPayloadPart> {
        let part: &dyn BedrockPayloadPart = &self.part;
        Some(part)
    }
}

struct OutputState {
    chunks: Mutex<VecDeque<String>>,
    block: Block,
    receives: AtomicUsize,
    receive_failure: Option<usize>,
}

struct ScriptedOutput {
    state: Arc<OutputState>,
}

#[async_trait]
impl BedrockOutputStream for ScriptedOutput {
    type Error = FakeError;

    async fn receive(&self) -> Result<Option<Box<dyn BedrockOutputChunk>>, StreamError<FakeError>> {
        let index = self.state.receives.fetch_add(1, Ordering::SeqCst);
        if self.state.receive_failure == Some(index) {
            return Err(stream_error(
                "the provider receive failed",
                "ModelStreamErrorException",
            ));
        }
        let chunk = self
            .state
            .chunks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        let Some(text) = chunk else {
            if self.state.block == Block::Stream {
                return pending().await;
            }
            return Ok(None);
        };
        let value: Box<dyn BedrockOutputChunk> = Box::new(ScriptedValue {
            part: ScriptedPart {
                bytes: Some(text.into_bytes()),
            },
        });
        Ok(Some(value))
    }
}

pub(super) struct ScriptedStream {
    input: ScriptedInput,
    output: Arc<OutputState>,
}

impl ScriptedStream {
    pub(super) fn new(
        chunks: Vec<String>,
        block: Block,
        receive_failure: Option<usize>,
        send_failure: Option<usize>,
    ) -> Self {
        let sent = Arc::new(Mutex::new(Vec::new()));
        let closed = Arc::new(AtomicBool::new(false));
        Self {
            input: ScriptedInput {
                sent,
                send_failure,
                closed,
            },
            output: Arc::new(OutputState {
                chunks: Mutex::new(VecDeque::from(chunks)),
                block,
                receives: AtomicUsize::new(0),
                receive_failure,
            }),
        }
    }

    pub(super) fn profile(&self) -> StreamProfile {
        StreamProfile {
            sent: Arc::clone(&self.input.sent),
            closed: Arc::clone(&self.input.closed),
        }
    }
}

#[async_trait]
impl BedrockBidirectionalStream for ScriptedStream {
    type Error = FakeError;

    fn input_stream(&self) -> &dyn BedrockInputStream<Error = FakeError> {
        &self.input
    }

    async fn await_output(
        &self,
    ) -> Result<(Json, Box<dyn BedrockOutputStream<Error = FakeError>>), StreamError<FakeError>>
    {
        let output: Box<dyn BedrockOutputStream<Error = FakeError>> = Box::new(ScriptedOutput {
            state: Arc::clone(&self.output),
        });
        Ok((Json::Null, output))
    }
}
