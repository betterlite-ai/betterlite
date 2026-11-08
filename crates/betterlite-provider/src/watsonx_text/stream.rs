use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ChunkUsage {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TextGenerationChunk {
    pub text: String,
    pub is_finished: bool,
    pub finish_reason: Option<String>,
    pub usage: Option<ChunkUsage>,
}

#[derive(Debug, Deserialize)]
struct StreamChunkBody {
    #[serde(default)]
    results: Option<Vec<StreamResult>>,
}

#[derive(Debug, Deserialize)]
struct StreamResult {
    #[serde(default)]
    generated_text: Option<String>,
    #[serde(default)]
    stop_reason: Option<String>,
    #[serde(default)]
    input_token_count: Option<i64>,
    #[serde(default)]
    generated_token_count: Option<i64>,
}

// B2214's BaseModelResponseIterator._handle_string_chunk over one transport line
#[must_use]
pub fn decode_stream_line(line: &str) -> TextGenerationChunk {
    if line.contains("[DONE]") {
        return TextGenerationChunk {
            text: String::new(),
            is_finished: true,
            finish_reason: Some(String::from("stop")),
            usage: None,
        };
    }
    let stripped = line
        .strip_prefix("data: ")
        .or_else(|| line.strip_prefix("data:"))
        .unwrap_or(line);
    // serde also deserializes a struct from a JSON array, but only a parsed object reaches chunk_parser
    let parsed = if stripped.trim_start().starts_with('{') {
        serde_json::from_str::<StreamChunkBody>(stripped).ok()
    } else {
        None
    };
    let Some(body) = parsed else {
        return TextGenerationChunk {
            text: String::new(),
            is_finished: false,
            finish_reason: Some(String::new()),
            usage: None,
        };
    };
    chunk_from_body(body)
}

fn chunk_from_body(body: StreamChunkBody) -> TextGenerationChunk {
    let Some(result) = body.results.and_then(|results| results.into_iter().next()) else {
        return TextGenerationChunk {
            text: String::new(),
            is_finished: false,
            finish_reason: Some(String::from("stop")),
            usage: None,
        };
    };
    let prompt_tokens = result.input_token_count.unwrap_or(0);
    let completion_tokens = result.generated_token_count.unwrap_or(0);
    TextGenerationChunk {
        text: result.generated_text.unwrap_or_default(),
        is_finished: result.stop_reason.as_deref() != Some("not_finished"),
        finish_reason: result.stop_reason,
        usage: Some(ChunkUsage {
            prompt_tokens,
            completion_tokens,
            total_tokens: prompt_tokens.saturating_add(completion_tokens),
        }),
    }
}
