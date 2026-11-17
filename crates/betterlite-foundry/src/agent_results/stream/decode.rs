use std::collections::VecDeque;
use std::mem;

use betterlite_http::error::HttpError;
use betterlite_openai::aleph_alpha::lines::LineDecoder;
use betterlite_prompts::arguments::json::Object;
use futures_util::StreamExt as _;
use futures_util::stream::{BoxStream, empty};

use super::super::error::AgentResultError;
use super::super::event::{AgentEvent, decode_agent_event};
use super::super::messages::{ASSISTANT_ROLE, TEXT_TYPE, transform_annotations};
use super::super::reply::STOP_FINISH_REASON;
use super::{
    AgentChunk, CHUNK_OBJECT, ChunkChoice, ChunkDelta, DATA_PREFIX, DONE_DATA, EVENT_PREFIX,
    MAX_LINE_BYTES, MESSAGE_COMPLETED_EVENT, MESSAGE_DELTA_EVENT, StreamIdentity,
    THREAD_CREATED_EVENT,
};

// LiteLLM trims every line, keeps current_event across blank lines and parses each data line on its own
pub(super) struct ChunkState {
    body: BoxStream<'static, Result<Vec<u8>, HttpError>>,
    decoder: LineDecoder,
    lines: VecDeque<String>,
    ready: VecDeque<AgentChunk>,
    model: String,
    identity: StreamIdentity,
    current_event: String,
    thread_id: Option<String>,
    annotations: Vec<Object>,
    failure: Option<AgentResultError>,
    done: bool,
    ended: bool,
}

impl ChunkState {
    pub(super) fn new(
        body: BoxStream<'static, Result<Vec<u8>, HttpError>>,
        model: &str,
        identity: StreamIdentity,
    ) -> Self {
        Self {
            body,
            decoder: LineDecoder::new(MAX_LINE_BYTES),
            lines: VecDeque::new(),
            ready: VecDeque::new(),
            model: model.to_owned(),
            identity,
            current_event: String::new(),
            thread_id: None,
            annotations: Vec::new(),
            failure: None,
            done: false,
            ended: false,
        }
    }

    pub(super) async fn advance(mut self) -> Option<(Result<AgentChunk, AgentResultError>, Self)> {
        loop {
            if let Some(chunk) = self.ready.pop_front() {
                return Some((Ok(chunk), self));
            }
            if self.done {
                return None;
            }
            if let Some(line) = self.lines.pop_front() {
                self.handle(&line);
                continue;
            }
            if let Some(error) = self.failure.take() {
                self.ended = true;
                return Some((Err(error), self));
            }
            if self.ended {
                return None;
            }
            self.fill().await;
        }
    }

    async fn fill(&mut self) {
        match self.body.next().await {
            Some(Ok(bytes)) => {
                if self.decoder.push(&bytes, &mut self.lines).is_err() {
                    self.failure = Some(AgentResultError::LineTooLong {
                        limit: MAX_LINE_BYTES,
                    });
                }
            }
            Some(Err(error)) => self.failure = Some(AgentResultError::Body(error)),
            None => {
                self.ended = true;
                self.decoder.flush(&mut self.lines);
            }
        }
    }

    fn handle(&mut self, line: &str) {
        let trimmed = line.trim();
        if let Some(event) = trimmed.strip_prefix(EVENT_PREFIX) {
            event.trim().clone_into(&mut self.current_event);
            return;
        }
        let Some(data) = trimmed.strip_prefix(DATA_PREFIX) else {
            return;
        };
        let payload = data.trim();
        if payload == DONE_DATA {
            self.finish();
            return;
        }
        let Ok(event) = decode_agent_event(payload.as_bytes()) else {
            return;
        };
        let kind = self.current_event.clone();
        match kind.as_str() {
            THREAD_CREATED_EVENT => self.record_thread(&event),
            MESSAGE_COMPLETED_EVENT => self.collect_annotations(event),
            MESSAGE_DELTA_EVENT => self.push_deltas(event),
            _ => {}
        }
    }

    fn record_thread(&mut self, event: &AgentEvent) {
        if let Some(id) = event.fields.get("id") {
            self.thread_id = id.as_str().map(str::to_owned);
        }
    }

    fn collect_annotations(&mut self, event: AgentEvent) {
        for item in event.content.unwrap_or_default() {
            if item.kind.as_deref() != Some(TEXT_TYPE) {
                continue;
            }
            let raw = item
                .text
                .as_ref()
                .and_then(|block| block.annotations.as_deref());
            if let Some(annotations) = transform_annotations(raw) {
                self.annotations.extend(annotations);
            }
        }
    }

    fn push_deltas(&mut self, event: AgentEvent) {
        let Some(delta) = event.delta else {
            return;
        };
        for item in delta.content.unwrap_or_default() {
            if item.kind.as_deref() != Some(TEXT_TYPE) {
                continue;
            }
            let Some(text) = item.text.as_ref().and_then(|block| block.value.as_deref()) else {
                continue;
            };
            if text.is_empty() {
                continue;
            }
            let chunk = self.delta_chunk(text);
            self.ready.push_back(chunk);
        }
    }

    fn delta_chunk(&self, text: &str) -> AgentChunk {
        AgentChunk {
            id: self.identity.id.clone(),
            created: self.identity.created,
            model: self.model.clone(),
            object: CHUNK_OBJECT,
            choices: vec![ChunkChoice {
                index: 0,
                finish_reason: None,
                delta: ChunkDelta {
                    content: Some(text.to_owned()),
                    role: Some(ASSISTANT_ROLE),
                    annotations: None,
                },
            }],
            thread_id: self.thread_id.clone().filter(|id| !id.is_empty()),
        }
    }

    fn finish(&mut self) {
        let annotations = if self.annotations.is_empty() {
            None
        } else {
            Some(mem::take(&mut self.annotations))
        };
        let chunk = AgentChunk {
            id: self.identity.id.clone(),
            created: self.identity.created,
            model: self.model.clone(),
            object: CHUNK_OBJECT,
            choices: vec![ChunkChoice {
                index: 0,
                finish_reason: Some(STOP_FINISH_REASON),
                delta: ChunkDelta {
                    content: None,
                    role: None,
                    annotations,
                },
            }],
            thread_id: self.thread_id.clone().filter(|id| !id.is_empty()),
        };
        self.ready.push_back(chunk);
        self.done = true;
        self.body = empty().boxed();
    }
}

mod tests;
