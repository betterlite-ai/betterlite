use std::num::FpCategory;

use serde::de::{Error as _, IgnoredAny};
use serde::{Deserialize, Serialize};
use serde_json::error::Category;

use super::error::TritonError;
use super::reply::ObjectOnly;

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

// LiteLLM runs `elif stripped_json_chunk:` and then chunk.get, so a falsy value is an empty chunk and a truthy non-object fails
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum StreamLine {
    Object(ObjectOnly<StreamObject>),
    Null(()),
    Flag(bool),
    Float(f64),
    Text(String),
    List(Vec<IgnoredAny>),
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
    let parsed = match serde_json::from_str::<StreamLine>(stripped) {
        Ok(StreamLine::Object(ObjectOnly { inner })) => inner,
        Ok(StreamLine::Null(()) | StreamLine::Flag(false)) => {
            return Ok(StreamChunk::default());
        }
        Ok(StreamLine::Float(number)) if number.classify() == FpCategory::Zero => {
            return Ok(StreamChunk::default());
        }
        Ok(StreamLine::Text(text)) if text.is_empty() => return Ok(StreamChunk::default()),
        Ok(StreamLine::List(items)) if items.is_empty() => return Ok(StreamChunk::default()),
        Ok(_) => {
            return Err(TritonError::ReplyShape {
                source: serde_json::Error::custom("expected a JSON object"),
            });
        }
        Err(source) => match source.classify() {
            Category::Syntax | Category::Eof => return Ok(StreamChunk::default()),
            Category::Data | Category::Io => return Err(TritonError::ReplyShape { source }),
        },
    };
    Ok(StreamChunk {
        text: parsed.text_output,
        is_finished: parsed.is_finished,
        finish_reason: parsed.stop_reason,
        index: parsed.index,
    })
}
