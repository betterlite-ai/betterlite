use std::num::TryFromIntError;
use std::str::FromStr as _;

use async_trait::async_trait;
use betterlite_core::agents::json::AgentJson;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_tokenizer::a2a_text::request_utils::{
    count_tokens as count_text_tokens, extract_text_from_response,
};
use betterlite_tokenizer::a2a_text::{A2aResponse, A2aTextError};
use betterlite_tokenizer::count::Counter;
use chrono::{DateTime, FixedOffset};
use futures_core::Stream;
use futures_util::StreamExt as _;
use indexmap::IndexMap;
use serde_json::Number;
use thiserror::Error;

use crate::client::records::compat::{Message, Part, SendMessageRequest};
use crate::client::records::response::SendMessageResponse;
use crate::cost_calculator::{CostCalculatorError, a2a_call_cost};
use crate::logging_context::{LoggingContextError, json_rpc_id_json, usage_json};

mod tests;

pub trait StreamedChunk {
    fn wire(&self) -> Result<AgentJson, serde_json::Error>;
}

impl StreamedChunk for SendMessageResponse {
    fn wire(&self) -> Result<AgentJson, serde_json::Error> {
        serde_json::to_value(self)
    }
}

#[async_trait]
pub trait TokenCounter: Send + Sync {
    async fn count_tokens(&self, text: &str) -> usize;
}

#[async_trait]
impl TokenCounter for Counter {
    async fn count_tokens(&self, text: &str) -> usize {
        count_text_tokens(self, text).await
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamSuccessCall {
    pub result: Json,
    pub start_time: DateTime<FixedOffset>,
    pub end_time: DateTime<FixedOffset>,
    pub cache_hit: Option<bool>,
    pub prefer_async_handlers: bool,
}

#[async_trait]
pub trait StreamSuccessDispatch: Send {
    async fn dispatch(&mut self, logging: &mut LoggingState, call: StreamSuccessCall);
}

// the transport's response cap for an agent's streamed reply
pub const DEFAULT_MAX_RETAINED_BYTES: usize = 0x80_0000;

pub struct StreamSetup<'counter, Dispatch, Clock> {
    pub counter: &'counter dyn TokenCounter,
    pub max_retained_bytes: usize,
    pub dispatch: Dispatch,
    pub clock: Clock,
}

#[derive(Debug, Error)]
pub enum StreamCompletionError {
    #[error(transparent)]
    Logging(#[from] LoggingContextError),
    #[error(transparent)]
    Cost(#[from] CostCalculatorError),
    #[error("the A2A streaming token count does not fit a u64")]
    TokenCount(#[source] TryFromIntError),
    #[error("the A2A streaming chunk is not valid JSON")]
    Chunk(#[from] serde_json::Error),
    #[error("the A2A streaming chunk text cannot be extracted")]
    Text(#[from] A2aTextError),
    #[error("the A2A streaming cost is not a JSON number")]
    CostNumber(#[source] serde_json::Error),
}

pub struct A2AStreamingIterator<'counter, Chunk, Source, Dispatch, Clock> {
    pub stream: Source,
    pub request: SendMessageRequest,
    pub logging: LoggingState,
    pub counter: &'counter dyn TokenCounter,
    pub dispatch: Dispatch,
    pub clock: Clock,
    pub start_time: DateTime<FixedOffset>,
    pub max_retained_bytes: usize,
    pub retained_bytes: usize,
    pub retention_capped: bool,
    pub chunks: Vec<Chunk>,
    pub collected_text_parts: Vec<String>,
    pub last_text: Option<String>,
    pub last_chunk: Option<Chunk>,
    pub final_chunk: Option<Chunk>,
    pub finished: bool,
}

impl<'counter, Chunk, Source, Dispatch, Clock>
    A2AStreamingIterator<'counter, Chunk, Source, Dispatch, Clock>
where
    Chunk: StreamedChunk + Clone,
    Dispatch: StreamSuccessDispatch,
    Clock: FnMut() -> DateTime<FixedOffset>,
{
    pub fn new(
        stream: Source,
        request: SendMessageRequest,
        logging: LoggingState,
        mut setup: StreamSetup<'counter, Dispatch, Clock>,
    ) -> Self {
        let start_time = (setup.clock)();
        Self {
            stream,
            request,
            logging,
            counter: setup.counter,
            dispatch: setup.dispatch,
            clock: setup.clock,
            start_time,
            max_retained_bytes: setup.max_retained_bytes,
            retained_bytes: 0,
            retention_capped: false,
            chunks: Vec::new(),
            collected_text_parts: Vec::new(),
            last_text: None,
            last_chunk: None,
            final_chunk: None,
            finished: false,
        }
    }

    pub async fn next_chunk<Error>(&mut self) -> Option<Result<Chunk, Error>>
    where
        Source: Stream<Item = Result<Chunk, Error>> + Unpin,
    {
        let item = self.stream.next().await;
        match item {
            Some(Ok(chunk)) => {
                self.collect(&chunk);
                Some(Ok(chunk))
            }
            Some(Err(error)) => Some(Err(error)),
            None => {
                if !self.finished {
                    self.finished = true;
                    self.finish().await;
                }
                None
            }
        }
    }

    fn collect(&mut self, chunk: &Chunk) {
        self.last_chunk = Some(chunk.clone());
        let Ok(wire) = chunk.wire() else {
            tracing::debug!("Failed to extract text from A2A streaming chunk");
            self.retain(chunk, size_of::<Chunk>(), None);
            return;
        };
        let text = match chunk_text(&wire) {
            Ok(text) if !text.is_empty() => Some(text),
            Ok(_) => None,
            Err(error) => {
                tracing::debug!("Failed to extract text from A2A streaming chunk: {error}");
                None
            }
        };
        if completed_chunk(&wire) {
            self.final_chunk = Some(chunk.clone());
        }
        if text.is_some() {
            self.last_text.clone_from(&text);
        }
        let text_len = text.as_ref().map_or(0, String::len);
        let cost = wire.to_string().len().saturating_add(text_len);
        self.retain(chunk, cost, text);
    }

    // an agent's stream is untrusted input, so the kept copies stop at a byte cap and the stream goes on
    fn retain(&mut self, chunk: &Chunk, cost: usize, text: Option<String>) {
        if self.retention_capped {
            return;
        }
        let total = self.retained_bytes.saturating_add(cost);
        if total > self.max_retained_bytes {
            self.retention_capped = true;
            tracing::warn!("A2A streaming retention cap reached, chunks are forwarded unkept");
            return;
        }
        self.retained_bytes = total;
        self.chunks.push(chunk.clone());
        if let Some(kept) = text {
            self.collected_text_parts.push(kept);
        }
    }

    async fn finish(&mut self) {
        if self.final_chunk.is_none() {
            self.final_chunk = self.last_chunk.take();
        }
        if let Err(error) = self.handle_stream_complete().await {
            tracing::debug!("Error in A2A streaming completion handler: {error}");
        }
    }

    async fn handle_stream_complete(&mut self) -> Result<(), StreamCompletionError> {
        let end_time = (self.clock)();
        let prompt_text = input_text(&self.request.params.message);
        let prompt_tokens = u64::try_from(self.counter.count_tokens(&prompt_text).await)
            .map_err(StreamCompletionError::TokenCount)?;
        let output_text = self.last_text.as_deref().unwrap_or("");
        let completion_tokens = u64::try_from(self.counter.count_tokens(output_text).await)
            .map_err(StreamCompletionError::TokenCount)?;
        let usage = usage_json(prompt_tokens, completion_tokens)?;
        self.logging
            .model_call_details
            .insert("usage".to_owned(), usage.clone());
        self.logging
            .model_call_details
            .insert("stream".to_owned(), Json::Bool(false));
        let cost = a2a_call_cost(Some(&self.logging))?;
        let cost_value =
            Number::from_str(&cost.to_string()).map_err(StreamCompletionError::CostNumber)?;
        self.logging
            .model_call_details
            .insert("response_cost".to_owned(), Json::Number(cost_value));
        let result = self.build_logging_result(&usage);
        let call = StreamSuccessCall {
            result,
            start_time: self.start_time,
            end_time,
            cache_hit: None,
            prefer_async_handlers: true,
        };
        self.dispatch.dispatch(&mut self.logging, call).await;
        tracing::info!(
            prompt_tokens,
            completion_tokens,
            response_cost = %cost,
            "A2A streaming completed"
        );
        Ok(())
    }

    fn build_logging_result(&self, usage: &Json) -> Json {
        let mut result = IndexMap::new();
        result.insert("id".to_owned(), json_rpc_id_json(&self.request.id));
        result.insert("jsonrpc".to_owned(), Json::Text("2.0".to_owned()));
        result.insert("usage".to_owned(), usage.clone());
        // LiteLLM drops the final result when the chunk cannot be dumped, and still dispatches
        if let Some(final_chunk) = &self.final_chunk
            && let Ok(wire) = final_chunk.wire()
            && let Some(chunk_result) = wire.get("result")
            && let Ok(converted) = serde_json::from_value(chunk_result.clone())
        {
            result.insert("result".to_owned(), converted);
        }
        Json::Object(result)
    }
}

fn input_text(message: &Message) -> String {
    let parts = message.parts.iter().filter_map(|part| match part {
        Part::Text(text) => Some(text.text.as_str()),
        Part::Data(_) | Part::File(_) => None,
    });
    parts.collect::<Vec<_>>().join(" ")
}

fn chunk_text(wire: &AgentJson) -> Result<String, StreamCompletionError> {
    match serde_json::from_value::<A2aResponse>(wire.clone()) {
        Ok(response) => Ok(extract_text_from_response(&response)?),
        Err(error) => Err(StreamCompletionError::Chunk(error)),
    }
}

fn completed_chunk(wire: &AgentJson) -> bool {
    let state = wire
        .get("result")
        .and_then(AgentJson::as_object)
        .and_then(|result| result.get("status"))
        .and_then(AgentJson::as_object)
        .and_then(|status| status.get("state"))
        .and_then(AgentJson::as_str);
    state == Some("completed")
}
