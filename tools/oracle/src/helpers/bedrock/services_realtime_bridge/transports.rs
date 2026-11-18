use std::collections::VecDeque;
use std::future::pending;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_bedrock::realtime_events::client::{ClientError, RealtimeClientWebSocket};
use betterlite_bedrock::realtime_events::stream::{
    BedrockBidirectionalStream, BedrockInputStream, BedrockOutputChunk, BedrockOutputStream,
    BedrockPayloadPart, StreamError,
};
use betterlite_models::decisions::Json;

use super::{Chunk, ChunkKind, Run};

pub(super) struct ClientState {
    frames: Mutex<VecDeque<Option<String>>>,
    block: bool,
    client_send_failures: Vec<usize>,
    sent: Mutex<Vec<String>>,
    closes: Mutex<Vec<(u16, Option<String>)>>,
    pub(super) receives: AtomicUsize,
}

// Records the value and answers the index when the case asked this send to fail
fn push_sent<T>(sent: &Mutex<Vec<T>>, failures: &[usize], value: T) -> Option<usize> {
    let mut sent = sent.lock().unwrap_or_else(PoisonError::into_inner);
    let index = sent.len();
    sent.push(value);
    failures.contains(&index).then_some(index)
}

impl ClientState {
    pub(super) fn sent(&self) -> Vec<String> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn closes(&self) -> Vec<(u16, Option<String>)> {
        self.closes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

pub(super) struct TransportClient {
    pub(super) state: Arc<ClientState>,
}

#[async_trait]
impl RealtimeClientWebSocket for TransportClient {
    type Error = io::Error;

    async fn receive_text(&self) -> Result<String, ClientError<io::Error>> {
        self.state.receives.fetch_add(1, Ordering::SeqCst);
        let frame = self
            .state
            .frames
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        if let Some(Some(text)) = frame {
            return Ok(text);
        }
        if self.state.block {
            return pending().await;
        }
        Err(ClientError {
            message: "the client is gone".to_owned(),
            source: None,
        })
    }

    async fn send_text(&self, data: String) -> Result<(), ClientError<io::Error>> {
        let failed = push_sent(&self.state.sent, &self.state.client_send_failures, data);
        failed.map_or(Ok(()), |_index| {
            Err(ClientError {
                message: "the client send failed".to_owned(),
                source: None,
            })
        })
    }

    async fn close(&self, code: u16, reason: Option<String>) {
        self.state
            .closes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((code, reason));
    }
}

pub(super) struct StreamState {
    sent: Mutex<Vec<Json>>,
    bedrock_send_failures: Vec<usize>,
    pub(super) closed: AtomicBool,
    close_error: bool,
    await_error: Option<String>,
    chunks: Mutex<VecDeque<Chunk>>,
    pub(super) receives: AtomicUsize,
    receive_failures: Vec<usize>,
    block: bool,
}

impl StreamState {
    pub(super) fn sent(&self) -> Vec<Json> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

pub(super) fn states(run: &Run) -> (Arc<ClientState>, Arc<StreamState>) {
    let client = ClientState {
        frames: Mutex::new(run.frames.iter().cloned().collect()),
        block: run.block.0,
        client_send_failures: run.client_send_failures.clone(),
        sent: Mutex::new(Vec::new()),
        closes: Mutex::new(Vec::new()),
        receives: AtomicUsize::new(0),
    };
    let wire = StreamState {
        sent: Mutex::new(Vec::new()),
        bedrock_send_failures: run.bedrock_send_failures.clone(),
        closed: AtomicBool::new(false),
        close_error: run.bedrock_close_error.0,
        await_error: run.await_error.clone(),
        chunks: Mutex::new(run.chunks.iter().cloned().collect()),
        receives: AtomicUsize::new(0),
        receive_failures: run.receive_failures.clone(),
        block: run.block.0,
    };
    (Arc::new(client), Arc::new(wire))
}

struct TransportInput {
    state: Arc<StreamState>,
}

#[async_trait]
impl BedrockInputStream for TransportInput {
    type Error = io::Error;

    async fn send(&self, event: Json) -> Result<(), StreamError<io::Error>> {
        if push_sent(&self.state.sent, &self.state.bedrock_send_failures, event).is_none() {
            return Ok(());
        }
        Err(StreamError {
            message: "the input send failed".to_owned(),
            source: None,
        })
    }

    async fn close(&self) -> Result<(), StreamError<io::Error>> {
        self.state.closed.store(true, Ordering::SeqCst);
        if self.state.close_error {
            return Err(StreamError {
                message: "the input close failed".to_owned(),
                source: None,
            });
        }
        Ok(())
    }
}

struct Part(Option<Vec<u8>>);

impl BedrockPayloadPart for Part {
    fn bytes(&self) -> Option<&[u8]> {
        self.0.as_deref()
    }
}

struct WireChunk(Option<Part>);

impl BedrockOutputChunk for WireChunk {
    fn value(&self) -> Option<&dyn BedrockPayloadPart> {
        let part: &dyn BedrockPayloadPart = self.0.as_ref()?;
        Some(part)
    }
}

struct TransportOutput {
    state: Arc<StreamState>,
}

#[async_trait]
impl BedrockOutputStream for TransportOutput {
    type Error = io::Error;

    async fn receive(&self) -> Result<Option<Box<dyn BedrockOutputChunk>>, StreamError<io::Error>> {
        let index = self.state.receives.fetch_add(1, Ordering::SeqCst);
        if self.state.receive_failures.contains(&index) {
            return Err(StreamError {
                message: "the provider receive failed".to_owned(),
                source: None,
            });
        }
        let chunk = self
            .state
            .chunks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        match chunk {
            Some(chunk) => match chunk.kind {
                ChunkKind::End => Ok(None),
                ChunkKind::MissingValue => Ok(Some(Box::new(WireChunk(None)))),
                ChunkKind::MissingBytes => Ok(Some(Box::new(WireChunk(Some(Part(None)))))),
                ChunkKind::Empty => Ok(Some(Box::new(WireChunk(Some(Part(Some(Vec::new()))))))),
                ChunkKind::Bytes => Ok(Some(Box::new(WireChunk(Some(Part(Some(
                    chunk.text.into_bytes(),
                ))))))),
                ChunkKind::InvalidUtf8 => {
                    Ok(Some(Box::new(WireChunk(Some(Part(Some(vec![0xff])))))))
                }
            },
            None if self.state.block => pending().await,
            None => Ok(None),
        }
    }
}

pub(super) struct TransportStream {
    state: Arc<StreamState>,
    input: TransportInput,
}

impl TransportStream {
    pub(super) fn new(state: Arc<StreamState>) -> Self {
        let input = TransportInput {
            state: Arc::clone(&state),
        };
        Self { input, state }
    }
}

#[async_trait]
impl BedrockBidirectionalStream for TransportStream {
    type Error = io::Error;

    fn input_stream(&self) -> &dyn BedrockInputStream<Error = io::Error> {
        &self.input
    }

    async fn await_output(
        &self,
    ) -> Result<(Json, Box<dyn BedrockOutputStream<Error = io::Error>>), StreamError<io::Error>>
    {
        if let Some(message) = &self.state.await_error {
            return Err(StreamError {
                message: message.clone(),
                source: None,
            });
        }
        Ok((
            Json::Null,
            Box::new(TransportOutput {
                state: Arc::clone(&self.state),
            }),
        ))
    }
}
