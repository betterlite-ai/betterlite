use betterlite_openai::openai_base_config::Object;
use serde::{Deserialize, Serialize};
use serde_json::Number;
use serde_with::rust::double_option;

use super::error::TranscriptionError;

pub const MAX_REPLY_BYTES: usize = 0x0100_0000;
const BYTE_ORDER_MARK: &[u8] = &[0xEF, 0xBB, 0xBF];

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
    words: Option<Option<Vec<Object>>>,
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
    // httpx decodes reply bytes as utf-8-sig, so json.loads skips a leading byte order mark
    let payload = body.strip_prefix(BYTE_ORDER_MARK).unwrap_or(body);
    // LiteLLM calls .json() once and then .get() on it, so the reply must be one JSON object and nothing else
    let hidden: Object = serde_json::from_slice(payload).map_err(&failed)?;
    let reply: ProviderReply = serde_json::from_slice(payload).map_err(&failed)?;
    let words = match reply.words {
        None => None,
        Some(None) => {
            return Err(TranscriptionError::Reply {
                message: "the words field is not a list".to_owned(),
                raw: raw(),
            });
        }
        Some(Some(entries)) => {
            let mut kept = Vec::new();
            for entry in entries {
                if entry.get("type").and_then(|value| value.as_str()) != Some("word") {
                    continue;
                }
                let projected =
                    project_word(&entry).map_err(|message| TranscriptionError::Reply {
                        message: message.to_owned(),
                        raw: raw(),
                    })?;
                kept.push(projected);
            }
            Some(kept)
        }
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

fn project_word(entry: &Object) -> Result<TranscriptWord, &'static str> {
    let word = match entry.get("text") {
        None => Some(String::new()),
        Some(value) if value.is_null() => None,
        Some(value) => Some(
            value
                .as_str()
                .ok_or("the word text is not a string")?
                .to_owned(),
        ),
    };
    let start = match entry.get("start") {
        None => Some(Number::from(0_u8)),
        Some(value) if value.is_null() => None,
        Some(value) => Some(
            value
                .as_number()
                .cloned()
                .ok_or("the word start is not a number")?,
        ),
    };
    let end = match entry.get("end") {
        None => Some(Number::from(0_u8)),
        Some(value) if value.is_null() => None,
        Some(value) => Some(
            value
                .as_number()
                .cloned()
                .ok_or("the word end is not a number")?,
        ),
    };
    Ok(TranscriptWord { word, start, end })
}
