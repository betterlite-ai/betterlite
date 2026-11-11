use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_bedrock::agentcore_stream::transport::{
    Chunks, HandlerKind, PostRequest, StreamHandler, StreamHead,
};
use betterlite_bedrock::agentcore_stream::{
    AgentCoreStream, AgentCoreStreamError, IdSource, StreamLogger, StreamPostCall,
};
use betterlite_http::error::HttpError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::stream::chunk::Chunk;
use futures_util::StreamExt as _;
use futures_util::stream;
use http::{HeaderMap, HeaderName, HeaderValue};
use uuid::Uuid;

use super::Scenario;
use super::failure;
use crate::helpers::HelperError;

pub(super) struct FixedIds {
    values: Mutex<VecDeque<Uuid>>,
}

impl FixedIds {
    pub(super) fn new(values: &[Uuid]) -> Self {
        Self {
            values: Mutex::new(values.iter().copied().collect()),
        }
    }
}

impl IdSource for FixedIds {
    fn next_id(&self) -> Uuid {
        let mut values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        values.pop_front().unwrap_or_else(Uuid::nil)
    }
}

pub(super) struct FixtureHandler {
    kind: HandlerKind,
    status: u16,
    headers: HeaderMap,
    reads: Vec<String>,
}

impl FixtureHandler {
    pub(super) fn new(kind: HandlerKind, scenario: &Scenario) -> Result<Self, Box<HelperError>> {
        Ok(Self {
            kind,
            status: scenario.status,
            headers: header_map(&scenario.headers)?,
            reads: scenario.reads.clone(),
        })
    }
}

#[async_trait]
impl StreamHandler for FixtureHandler {
    fn kind(&self) -> HandlerKind {
        self.kind
    }

    async fn post(&self, _request: PostRequest) -> Result<StreamHead, TransportError> {
        Ok(StreamHead {
            status: self.status,
            headers: self.headers.clone(),
            chunks: chunk_stream(&self.reads),
        })
    }
}

pub(super) fn header_map(pairs: &[(String, String)]) -> Result<HeaderMap, Box<HelperError>> {
    let mut headers = HeaderMap::new();
    for (name, value) in pairs {
        let header_name =
            HeaderName::from_bytes(name.as_bytes()).map_err(|error| failure(error.to_string()))?;
        let header_value =
            HeaderValue::from_str(value).map_err(|error| failure(error.to_string()))?;
        headers.append(header_name, header_value);
    }
    Ok(headers)
}

fn chunk_stream(reads: &[String]) -> Chunks {
    let items: Vec<Result<Vec<u8>, HttpError>> = reads
        .iter()
        .map(|read| Ok(read.clone().into_bytes()))
        .collect();
    stream::iter(items).boxed()
}

pub(super) struct NoLogger;

impl StreamLogger for NoLogger {
    fn post_call(&self, _call: StreamPostCall<'_>) {}
}

pub(super) async fn drain(
    stream: &mut AgentCoreStream,
) -> Result<Vec<Chunk>, AgentCoreStreamError> {
    let mut chunks = Vec::new();
    loop {
        match stream.next_chunk().await {
            Ok(Some(chunk)) => chunks.push(chunk),
            Ok(None) => break Ok(chunks),
            Err(error) => break Err(error),
        }
    }
}
