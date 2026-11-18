use crate::realtime_events::client::{ClientError, RealtimeClientWebSocket};
use crate::realtime_events::stream::{
    BedrockBidirectionalStream, BedrockInputStream, BedrockOutputChunk, BedrockOutputStream,
    ClosableBedrockRuntimeClient, StreamError,
};
use async_trait::async_trait;
use betterlite_models::decisions::Json;
use std::collections::VecDeque;
use std::io;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

pub(super) enum Received {
    Text(String),
    Error(String),
}

pub(super) struct FakeClient {
    pub(super) received: Mutex<VecDeque<Received>>,
    pub(super) sent: Mutex<Vec<String>>,
    pub(super) send_error: Option<String>,
}

impl FakeClient {
    pub(super) fn new() -> Self {
        Self {
            received: Mutex::new(VecDeque::new()),
            sent: Mutex::new(Vec::new()),
            send_error: None,
        }
    }

    pub(super) fn sent(&self) -> Vec<String> {
        self.sent.lock().unwrap().clone()
    }
}

#[async_trait]
impl RealtimeClientWebSocket for FakeClient {
    type Error = io::Error;

    async fn receive_text(&self) -> Result<String, ClientError<io::Error>> {
        let mut received = self.received.lock().unwrap();
        match received.pop_front() {
            Some(Received::Text(text)) => Ok(text),
            Some(Received::Error(message)) => Err(ClientError {
                message,
                source: None,
            }),
            None => Err(ClientError {
                message: "no more messages".to_owned(),
                source: None,
            }),
        }
    }

    async fn send_text(&self, data: String) -> Result<(), ClientError<io::Error>> {
        self.sent.lock().unwrap().push(data);
        self.send_error.as_ref().map_or(Ok(()), |message| {
            Err(ClientError {
                message: message.clone(),
                source: None,
            })
        })
    }

    async fn close(&self, _code: u16, _reason: Option<String>) {}
}

pub(super) struct FakeInput;

#[async_trait]
impl BedrockInputStream for FakeInput {
    type Error = io::Error;

    async fn send(&self, _event: Json) -> Result<(), StreamError<io::Error>> {
        Ok(())
    }

    async fn close(&self) -> Result<(), StreamError<io::Error>> {
        Ok(())
    }
}

pub(super) struct FakeOutput;

#[async_trait]
impl BedrockOutputStream for FakeOutput {
    type Error = io::Error;

    async fn receive(&self) -> Result<Option<Box<dyn BedrockOutputChunk>>, StreamError<io::Error>> {
        Ok(None)
    }
}

pub(super) struct FakeStream {
    pub(super) input: FakeInput,
    pub(super) error: Option<String>,
    pub(super) awaited: AtomicU32,
}

impl FakeStream {
    pub(super) fn new() -> Self {
        Self {
            input: FakeInput,
            error: None,
            awaited: AtomicU32::new(0),
        }
    }

    pub(super) fn awaited(&self) -> u32 {
        self.awaited.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl BedrockBidirectionalStream for FakeStream {
    type Error = io::Error;

    fn input_stream(&self) -> &dyn BedrockInputStream<Error = io::Error> {
        &self.input
    }

    async fn await_output(
        &self,
    ) -> Result<(Json, Box<dyn BedrockOutputStream<Error = io::Error>>), StreamError<io::Error>>
    {
        self.awaited.fetch_add(1, Ordering::SeqCst);
        self.error.as_ref().map_or_else(
            || Ok((Json::Null, Box::new(FakeOutput))),
            |message| {
                Err(StreamError {
                    message: message.clone(),
                    source: None,
                })
            },
        )
    }
}

pub(super) struct Closable {
    pub(super) error: Option<String>,
}

#[async_trait]
impl ClosableBedrockRuntimeClient for Closable {
    type Error = io::Error;

    async fn close(&self) -> Result<(), StreamError<io::Error>> {
        let Some(message) = &self.error else {
            return Ok(());
        };
        Err(StreamError {
            message: message.clone(),
            source: None,
        })
    }
}
