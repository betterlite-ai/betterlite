use std::future::pending;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use thiserror::Error;

// the client link the admitted request carried; the worker holds one but never reads it
#[async_trait]
pub trait ClientInbound: Send + Sync {
    async fn receive(&mut self);
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

// background_streaming.py's detach_request_from_client: the scope stays, the hangup never arrives
#[derive(Debug)]
pub struct DetachedRequest<'request> {
    pub scope: &'request PollScope,
}

// background_streaming.py's _never_receive: waiting forever, so a closed client cancels nothing
#[async_trait]
impl ClientInbound for DetachedRequest<'_> {
    async fn receive(&mut self) {
        pending::<()>().await;
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
