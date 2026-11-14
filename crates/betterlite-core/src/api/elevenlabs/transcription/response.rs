use betterlite_openai::openai_base_config::Object;
use serde::de::IgnoredAny;
use serde::{Deserialize, Serialize};
use serde_json::Number;
use serde_with::rust::double_option;

use super::error::TranscriptionError;

pub const MAX_REPLY_BYTES: usize = 0x0100_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TranscriptTask {
    Transcribe,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TranscriptWord {
    pub word: Option<String>,
    pub start: Option<Number>,
    pub end: Option<Number>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ElevenTranscript {
    pub text: Option<String>,
    pub task: TranscriptTask,
    pub language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub words: Option<Vec<TranscriptWord>>,
    // LiteLLM stores the complete provider answer as hidden params, so its unmodelled keys ride along here
    #[serde(skip)]
    pub hidden: Object,
}

// an absent key takes LiteLLM's .get() default while a present null stays null
#[derive(Debug, Deserialize)]
struct ProviderReply {
    #[serde(default, with = "double_option")]
    text: Option<Option<String>>,
    #[serde(default, with = "double_option")]
    language_code: Option<Option<String>>,
    #[serde(default, with = "double_option")]
    words: Option<Option<Vec<ProviderWord>>>,
}

#[derive(Debug, Deserialize)]
struct ProviderWord {
    #[serde(rename = "type", default)]
    kind: WordKind,
    #[serde(default, with = "double_option")]
    text: Option<Option<String>>,
    #[serde(default, with = "double_option")]
    start: Option<Option<Number>>,
    #[serde(default, with = "double_option")]
    end: Option<Option<Number>>,
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
    let raw = || {
        str::from_utf8(body).map_or_else(
            |_| format!("{} bytes that are not UTF-8", body.len()),
            str::to_owned,
        )
    };
    let failed = |source: serde_json::Error| TranscriptionError::Reply {
        message: source.to_string(),
        raw: raw(),
    };
    // LiteLLM calls .json() once and then .get() on it, so the reply must be one JSON object and nothing else
    let hidden: Object = serde_json::from_slice(body).map_err(&failed)?;
    let reply: ProviderReply = serde_json::from_slice(body).map_err(&failed)?;
    let words = match reply.words {
        None => None,
        Some(None) => {
            return Err(TranscriptionError::Reply {
                message: "the words field is not a list".to_owned(),
                raw: raw(),
            });
        }
        Some(Some(words)) => Some(
            words
                .into_iter()
                .filter(|word| matches!(&word.kind, WordKind::Text(kind) if kind == "word"))
                .map(|word| TranscriptWord {
                    word: word.text.unwrap_or_else(|| Some(String::new())),
                    start: word.start.unwrap_or_else(|| Some(Number::from(0_u8))),
                    end: word.end.unwrap_or_else(|| Some(Number::from(0_u8))),
                })
                .collect(),
        ),
    };
    Ok(ElevenTranscript {
        text: reply.text.unwrap_or_else(|| Some(String::new())),
        task: TranscriptTask::Transcribe,
        language: reply
            .language_code
            .unwrap_or_else(|| Some("unknown".to_owned())),
        words,
        hidden,
    })
}
