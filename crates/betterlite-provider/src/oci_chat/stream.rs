use std::time::Duration;

use betterlite_openai::stream::chunk::Choice;
use betterlite_prompts::arguments::json::Json;

use crate::oci_chat_wire::cohere::reply::{CohereEmitted, handle_cohere_stream_chunk};
use crate::oci_chat_wire::generic::reply::handle_generic_stream_chunk;
use crate::oci_common::OciError;

pub const STREAMING_TIMEOUT: Duration = Duration::from_secs(300);

const DATA_PREFIX: &str = "data:";
const DONE_PAYLOAD: &str = "[DONE]";
const COHERE_FORMAT: &str = "COHERE";
const OK_STATUS: u16 = 200;

#[derive(Debug)]
pub struct StreamChunk {
    pub choice: Choice,
    pub tool_call: bool,
    pub finish_reason: Option<String>,
}

#[derive(Debug)]
pub enum StreamItem {
    Done,
    Chunk(StreamChunk),
}

#[derive(Debug, Default)]
pub struct SseDecoder {
    buffer: String,
    emitted: CohereEmitted,
}

impl SseDecoder {
    pub fn push(&mut self, text: &str) -> Result<Vec<StreamItem>, OciError> {
        self.buffer.push_str(text);
        let mut items = Vec::new();
        while let Some(index) = self.buffer.find('\n') {
            let rest = self.buffer.split_off(index.saturating_add(1));
            let line = std::mem::replace(&mut self.buffer, rest);
            self.decode_line(&line, &mut items)?;
        }
        Ok(items)
    }

    pub fn finish(&mut self) -> Result<Vec<StreamItem>, OciError> {
        let mut items = Vec::new();
        let tail = std::mem::take(&mut self.buffer);
        self.decode_line(&tail, &mut items)?;
        Ok(items)
    }

    fn decode_line(&mut self, line: &str, items: &mut Vec<StreamItem>) -> Result<(), OciError> {
        let Some(payload) = line.trim().strip_prefix(DATA_PREFIX) else {
            return Ok(());
        };
        let payload = payload.trim();
        if payload == DONE_PAYLOAD {
            items.push(StreamItem::Done);
            return Ok(());
        }
        let chunk: Json = serde_json::from_str(payload).map_err(|source| OciError {
            status: 500,
            message: format!("Chunk cannot be parsed as JSON: {source}"),
        })?;
        items.push(StreamItem::Chunk(self.decode_chunk(&chunk)?));
        Ok(())
    }

    fn decode_chunk(&mut self, chunk: &Json) -> Result<StreamChunk, OciError> {
        let cohere = chunk
            .as_object()
            .and_then(|fields| fields.get("apiFormat"))
            .and_then(Json::as_str)
            == Some(COHERE_FORMAT);
        let choice = if cohere {
            let choice = handle_cohere_stream_chunk(chunk, self.emitted)?;
            self.record(&choice);
            choice
        } else {
            handle_generic_stream_chunk(chunk.clone())?
        };
        let tool_call = choice.delta.tool_calls.is_some();
        let finish_reason = choice.finish_reason.clone();
        Ok(StreamChunk {
            choice,
            tool_call,
            finish_reason,
        })
    }

    fn record(&mut self, choice: &Choice) {
        if !self.emitted.tool_calls && choice.delta.tool_calls.is_some() {
            self.emitted.tool_calls = true;
        }
        if !self.emitted.text
            && choice
                .delta
                .content
                .as_deref()
                .is_some_and(|text| !text.is_empty())
        {
            self.emitted.text = true;
        }
    }
}

pub fn check_stream_status(status: u16, body: &str) -> Result<(), OciError> {
    if status == OK_STATUS {
        return Ok(());
    }
    Err(OciError {
        status,
        message: body.to_owned(),
    })
}
