use std::future::pending;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use thiserror::Error;

// what a client link can report; the worker holds one but never reads it
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientMessage {
    Body(Vec<u8>),
    Disconnect,
}

#[async_trait]
pub trait ClientInbound: Send + Sync {
    async fn receive(&mut self) -> ClientMessage;
}

// the preprocessed scope and body B080 copies when it admits the polling request
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PollScope {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

pub struct PollRequest {
    pub scope: PollScope,
    pub inbound: Box<dyn ClientInbound>,
}

impl PollRequest {
    // background_streaming.py's detach_request_from_client: the scope stays, the hangup never arrives
    #[must_use]
    pub fn detach(&self) -> DetachedRequest<'_> {
        DetachedRequest { scope: &self.scope }
    }
}

// the same scope the processor would see, with a receive that never reports http.disconnect
pub struct DetachedRequest<'request> {
    pub scope: &'request PollScope,
}

impl DetachedRequest<'_> {
    // background_streaming.py's _never_receive: waiting forever, so a closed client cancels nothing
    pub async fn receive(&self) -> ClientMessage {
        pending::<()>().await;
        ClientMessage::Disconnect
    }
}

// the provider call the worker makes, as the source fills base_process_llm_request's arguments
#[derive(Debug)]
pub struct PollProcessCall<'call> {
    pub route_type: &'static str,
    pub model: Option<String>,
    pub skip_pre_call_logic: bool,
    pub data: &'call IndexMap<String, Json>,
    pub request: DetachedRequest<'call>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct PollProcessorError {
    pub message: String,
}

#[async_trait]
pub trait PollRequestProcessor: Send + Sync {
    async fn process(&self, call: PollProcessCall<'_>)
    -> Result<PollProcessed, PollProcessorError>;
}

// a streaming answer hands back its body iterator; anything else is the source's misconfiguration
pub enum PollProcessed {
    Streaming(Box<dyn PollChunkSource>),
    NonStreaming,
}

// the body iterator of the streaming response, one chunk per call
#[async_trait]
pub trait PollChunkSource: Send {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, PollProcessorError>;
}
