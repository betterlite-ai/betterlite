use betterlite_openai::openai_base_config::Object;
use serde::{Deserialize, Serialize};
use serde_json::Number;
use serde_with::rust::double_option;
use thiserror::Error;

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

#[derive(Debug, Error)]
enum WordError {
    #[error("the words field is not a list")]
    NotList,
    #[error("the word text is not a string")]
    Text,
    #[error("the word start is not a number")]
    Start,
    #[error("the word end is not a number")]
    End,
}

pub fn decode_eleven_transcript(
    body: &[u8],
    max_bytes: usize,
) -> Result<ElevenTranscript, TranscriptionError> {
    if body.len() > max_bytes {
        return Err(TranscriptionError::ReplyTooLarge { limit: max_bytes });
    }
    // httpx decodes reply bytes as utf-8-sig, so json.loads skips a leading byte order mark
    let payload = body.strip_prefix(BYTE_ORDER_MARK).unwrap_or(body);
    // LiteLLM calls .json() once and then .get() on it, so the reply must be one JSON object and nothing else
    let hidden: Object =
        serde_json::from_slice(payload).map_err(|source| reply_error(body, source.to_string()))?;
    let reply: ProviderReply =
        serde_json::from_slice(payload).map_err(|source| reply_error(body, source.to_string()))?;
    let words = match reply.words {
        None => None,
        Some(None) => return Err(reply_error(body, WordError::NotList.to_string())),
        Some(Some(entries)) => {
            Some(project_words(&entries).map_err(|source| reply_error(body, source.to_string()))?)
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

fn reply_error(body: &[u8], message: String) -> TranscriptionError {
    TranscriptionError::Reply {
        message,
        raw: str::from_utf8(body).map_or_else(
            |_| format!("{} bytes that are not UTF-8", body.len()),
            str::to_owned,
        ),
    }
}

fn project_words(entries: &[Object]) -> Result<Vec<TranscriptWord>, WordError> {
    entries
        .iter()
        .filter(|entry| entry.get("type").and_then(|value| value.as_str()) == Some("word"))
        .map(project_word)
        .collect()
}

fn project_word(entry: &Object) -> Result<TranscriptWord, WordError> {
    let word = match entry.get("text") {
        None => Some(String::new()),
        Some(value) if value.is_null() => None,
        Some(value) => Some(value.as_str().ok_or(WordError::Text)?.to_owned()),
    };
    let start = match entry.get("start") {
        None => Some(Number::from(0_u8)),
        Some(value) if value.is_null() => None,
        Some(value) => Some(value.as_number().cloned().ok_or(WordError::Start)?),
    };
    let end = match entry.get("end") {
        None => Some(Number::from(0_u8)),
        Some(value) if value.is_null() => None,
        Some(value) => Some(value.as_number().cloned().ok_or(WordError::End)?),
    };
    Ok(TranscriptWord { word, start, end })
}
