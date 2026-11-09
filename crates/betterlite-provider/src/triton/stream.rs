use serde::{Deserialize, Serialize};
use serde_json::error::Category;

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
    let parsed: StreamObject = match serde_json::from_str(stripped) {
        Ok(object) => object,
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
