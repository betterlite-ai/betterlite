use betterlite_openai::stream::chunk::{Chunk, ChunkError};
use betterlite_prompts::arguments::json::{Json, Object};
use serde::Deserialize;
use thiserror::Error;

mod event;
mod tests;

const CHUNK_OBJECT: &str = "chat.completion.chunk";
const DEFAULT_CHUNK_ID: &str = "stream-chunk";
const DEFAULT_MODEL: &str = "unknown";
const DATA_PREFIX: &str = "data: ";
const DONE: &str = "[DONE]";
const ASYNC_ERROR_STATUS: u16 = 502;

#[derive(Debug, Error)]
#[error("{message}")]
pub struct SapOrchestrationError {
    pub status_code: u16,
    pub message: String,
}

#[derive(Debug, Error)]
pub enum SapStreamError {
    #[error("the SAP orchestration stream event reports an error: {message}")]
    Event { message: String },
    #[error("the SAP orchestration stream chunk is invalid: {source}")]
    Chunk {
        #[from]
        source: ChunkError,
    },
    #[error("{0} in the SAP orchestration stream event is invalid")]
    EventField(&'static str),
    #[error("the SAP orchestration stream cannot use {created} as a creation time")]
    Timestamp { created: i64 },
    #[error("the SAP orchestration request body must have a config object")]
    ConfigBody,
    #[error("config.stream in the SAP orchestration request body must be an object")]
    StreamBody,
    #[error(transparent)]
    Orchestration(#[from] SapOrchestrationError),
}

impl SapStreamError {
    fn for_mode(self, mode: Mode) -> Self {
        match mode {
            Mode::Sync => self,
            Mode::Async => Self::Orchestration(SapOrchestrationError {
                status_code: ASYNC_ERROR_STATUS,
                message: self.to_string(),
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Sync,
    Async,
}

#[derive(Debug)]
pub enum SapStep {
    Chunk(Box<Chunk>),
    Skipped,
    End,
}

#[derive(Debug)]
pub struct SapLineStream {
    pub now: i64,
    pub done: bool,
}

impl SapLineStream {
    pub fn next_chunk(&mut self, line: &str, mode: Mode) -> Result<SapStep, SapStreamError> {
        if self.done {
            return Ok(SapStep::End);
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Ok(SapStep::Skipped);
        }
        let payload = trimmed.strip_prefix(DATA_PREFIX).unwrap_or(trimmed);
        if payload == DONE {
            self.done = true;
            return Ok(SapStep::End);
        }
        let Ok(decoded) = serde_json::from_str::<Json>(payload) else {
            return Ok(SapStep::Skipped);
        };
        let found = match event_to_chunk(&decoded, self.now) {
            Ok(found) => found,
            Err(error) => {
                self.done = true;
                return Err(error.for_mode(mode));
            }
        };
        let Some(chunk) = found else {
            return Ok(SapStep::Skipped);
        };
        if chunk
            .choices
            .iter()
            .any(|choice| choice.finish_reason.is_some())
        {
            self.done = true;
        }
        Ok(SapStep::Chunk(Box::new(chunk)))
    }

    pub const fn finish(&mut self) -> SapStep {
        self.done = true;
        SapStep::End
    }

    pub fn read_failed<E>(&mut self, error: E, mode: Mode) -> Result<SapStep, E> {
        if mode == Mode::Sync {
            return Err(error);
        }
        self.done = true;
        Ok(SapStep::End)
    }
}

pub fn event_to_chunk(event: &Json, now: i64) -> Result<Option<Chunk>, SapStreamError> {
    let Json::Object(fields) = event else {
        return Ok(None);
    };
    if fields.contains_key("code") || fields.contains_key("error") {
        let message = serde_json::to_string(event).map_err(ChunkError::from)?;
        return Err(SapStreamError::Event { message });
    }
    if let Some(value) = fields.get("final_result") {
        return event::final_result_chunk(value).map(Some);
    }
    if let Some(value) = fields.get("orchestration_result") {
        return event::orchestration_chunk(value, fields, now);
    }
    if fields.contains_key("choices") && fields.contains_key("object") {
        return event::chunk_from_object(fields).map(Some);
    }
    Ok(None)
}

pub fn add_stream_flag(body: &mut Json) -> Result<(), SapStreamError> {
    let Json::Object(fields) = body else {
        return Err(SapStreamError::ConfigBody);
    };
    let Some(Json::Object(config)) = fields.get_mut("config") else {
        return Err(SapStreamError::ConfigBody);
    };
    match config.get_mut("stream") {
        Some(Json::Object(stream)) => {
            stream.insert(String::from("enabled"), Json::Bool(true));
        }
        None | Some(Json::Null) => {
            config.insert(
                String::from("stream"),
                Json::Object(Object::from([(String::from("enabled"), Json::Bool(true))])),
            );
        }
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(SapStreamError::StreamBody);
        }
    }
    Ok(())
}
