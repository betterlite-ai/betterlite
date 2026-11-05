use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod wire;
use crate::text_completion_policy::reply::mapped_finish_reason;
use wire::{Response, Tool, Usage};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct CompatibleLineChunk {
    pub text: String,
    pub tool_use: Option<Tool>,
    pub is_finished: bool,
    pub finish_reason: String,
    pub usage: Option<Usage>,
    pub index: u32,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Sync,
    Async,
}

#[derive(Debug, Error)]
pub enum ReceiveError<E> {
    #[error("{0}")]
    Value(#[source] E),
    #[error("{0}")]
    Other(#[source] E),
}

#[derive(Debug, Error)]
pub enum StreamError<E> {
    #[error("Error receiving chunk from stream: {source}")]
    Runtime {
        #[source]
        source: E,
    },
    #[error("{source}")]
    Other {
        #[source]
        source: E,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Outcome {
    Chunk(CompatibleLineChunk),
    End,
}

pub fn decode_legacy_compatible_line(line: &str, mode: Mode) -> Outcome {
    let content = line.strip_prefix("data:").unwrap_or(line).trim();
    if mode == Mode::Async && content == "[DONE]" {
        return Outcome::End;
    }
    if content.is_empty() {
        return Outcome::Chunk(CompatibleLineChunk::default());
    }
    match serde_json::from_str::<Response>(content) {
        Ok(response) => Outcome::Chunk(project(response)),
        Err(error) => {
            tracing::debug!(%error, "Error parsing chunk; defaulting to empty chunk");
            Outcome::Chunk(CompatibleLineChunk::default())
        }
    }
}

pub fn receive_failure<E>(error: ReceiveError<E>, mode: Mode) -> StreamError<E> {
    match (error, mode) {
        (ReceiveError::Value(source), Mode::Sync | Mode::Async)
        | (ReceiveError::Other(source), Mode::Async) => StreamError::Runtime { source },
        (ReceiveError::Other(source), Mode::Sync) => StreamError::Other { source },
    }
}

fn project(response: Response) -> CompatibleLineChunk {
    let mut chunk = CompatibleLineChunk {
        text: String::new(),
        tool_use: None,
        is_finished: false,
        finish_reason: String::new(),
        usage: response.usage,
        index: 0,
    };
    let Some(choice) = response.choices.into_iter().next() else {
        return chunk;
    };
    chunk.text = choice.delta.content.unwrap_or_default();
    if let Some(call) = choice.delta.tool_calls.into_iter().next() {
        chunk.tool_use = Some(Tool {
            id: call.id,
            kind: "function".to_owned(),
            function: call.function,
            index: call.index,
        });
    }
    if let Some(reason) = choice.finish_reason.filter(|reason| !reason.is_empty()) {
        chunk.is_finished = true;
        chunk.finish_reason = String::from(mapped_finish_reason(&reason));
    }
    chunk
}

mod tests;
