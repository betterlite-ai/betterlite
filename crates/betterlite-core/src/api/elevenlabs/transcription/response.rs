use betterlite_openai::openai_base_config::Object;
use serde::de::IgnoredAny;
use serde::{Deserialize, Serialize};
use serde_json::Number;

use super::error::TranscriptionError;

pub const MAX_REPLY_BYTES: usize = 0x0100_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TranscriptTask {
    Transcribe,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TranscriptWord {
    pub word: String,
    pub start: Number,
    pub end: Number,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ElevenTranscript {
    pub text: String,
    pub task: TranscriptTask,
    pub language: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub words: Option<Vec<TranscriptWord>>,
    // LiteLLM stores the complete provider answer as hidden params, so its unmodelled keys ride along here
    #[serde(skip)]
    pub hidden: Object,
}

#[derive(Debug, Deserialize)]
struct ProviderReply {
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    language_code: Option<String>,
    #[serde(default)]
    words: Option<Vec<ProviderWord>>,
}

#[derive(Debug, Deserialize)]
struct ProviderWord {
    #[serde(rename = "type", default)]
    kind: WordKind,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    start: Option<Number>,
    #[serde(default)]
    end: Option<Number>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum WordKind {
    Text(String),
    Other(IgnoredAny),
}

impl Default for WordKind {
    fn default() -> Self {
        Self::Other(IgnoredAny)
    }
}

pub fn decode_eleven_transcript(
    body: &[u8],
    max_bytes: usize,
) -> Result<ElevenTranscript, TranscriptionError> {
    if body.len() > max_bytes {
        return Err(TranscriptionError::ReplyTooLarge { limit: max_bytes });
    }
    let failed = |source: serde_json::Error| TranscriptionError::Reply {
        source,
        raw: str::from_utf8(body).map_or_else(
            |_| format!("{} bytes that are not UTF-8", body.len()),
            str::to_owned,
        ),
    };
    // LiteLLM calls .json() once and then .get() on it, so the reply must be one JSON object and nothing else
    let hidden: Object = serde_json::from_slice(body).map_err(&failed)?;
    let reply: ProviderReply = serde_json::from_slice(body).map_err(&failed)?;
    let words = reply.words.map(|words| {
        words
            .into_iter()
            .filter(|word| matches!(&word.kind, WordKind::Text(kind) if kind == "word"))
            .map(|word| TranscriptWord {
                word: word.text.unwrap_or_default(),
                start: word.start.unwrap_or_else(|| Number::from(0_u8)),
                end: word.end.unwrap_or_else(|| Number::from(0_u8)),
            })
            .collect()
    });
    Ok(ElevenTranscript {
        text: reply.text.unwrap_or_default(),
        task: TranscriptTask::Transcribe,
        language: reply.language_code.unwrap_or_else(|| "unknown".to_owned()),
        words,
        hidden,
    })
}
