use serde::de::{Error as _, IgnoredAny};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use super::error::TritonError;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct StreamChunk {
    pub text: String,
    pub is_finished: bool,
    pub finish_reason: String,
    pub index: i64,
}

#[derive(Debug, Deserialize)]
struct StreamObject {
    #[serde(default)]
    text_output: String,
    #[serde(default)]
    stop_reason: String,
    #[serde(default)]
    is_finished: bool,
    #[serde(default)]
    index: i64,
}

// B2214's BaseModelResponseIterator._handle_string_chunk over one transport line, then chunk_parser
pub fn decode_stream_line(line: &str) -> Result<StreamChunk, TritonError> {
    if line.contains("[DONE]") {
        return Ok(StreamChunk {
            text: String::new(),
            is_finished: true,
            finish_reason: String::from("stop"),
            index: 0,
        });
    }
    let stripped = line
        .strip_prefix("data: ")
        .or_else(|| line.strip_prefix("data:"))
        .unwrap_or(line);
    let Ok(raw) = serde_json::from_str::<Box<RawValue>>(stripped) else {
        return Ok(StreamChunk::default());
    };
    let text = raw.get();
    if text.starts_with('{') {
        let parsed = serde_json::from_str::<StreamObject>(text)
            .map_err(|source| TritonError::ReplyShape { source })?;
        return Ok(StreamChunk {
            text: parsed.text_output,
            is_finished: parsed.is_finished,
            finish_reason: parsed.stop_reason,
            index: parsed.index,
        });
    }
    // LiteLLM runs `elif stripped_json_chunk:` and then chunk.get, so a falsy value is an empty chunk and a truthy non-object fails
    let falsy = match text.as_bytes().first() {
        Some(b'"') => serde_json::from_str::<String>(text).is_ok_and(|value| value.is_empty()),
        Some(b'[') => {
            serde_json::from_str::<Vec<IgnoredAny>>(text).is_ok_and(|items| items.is_empty())
        }
        Some(b'n' | b'f') => true,
        _ => text.parse::<f64>().is_ok_and(|number| number == 0.0_f64),
    };
    if falsy {
        return Ok(StreamChunk::default());
    }
    Err(TritonError::ReplyShape {
        source: serde_json::Error::custom("expected a JSON object"),
    })
}
