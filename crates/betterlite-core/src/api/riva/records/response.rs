use betterlite_openai::api::transcription_contract::TranscriptionResponse;
use betterlite_openai::openai_base_config::Object;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::error::RecordsError;

pub const TRANSCRIBE_TASK: &str = "transcribe";
pub const VERBOSE_JSON: &str = "verbose_json";
pub const WORD_GRANULARITY: &str = "word";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WordTiming {
    pub word: String,
    pub start_time_ms: i64,
    pub end_time_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FinalResult {
    pub transcript: String,
    #[serde(default)]
    pub words: Vec<WordTiming>,
}

pub fn build_transcription_response(
    final_results: &[FinalResult],
    response_format: &str,
    duration_seconds: Option<f64>,
    timestamp_granularities: Option<&[String]>,
) -> Result<TranscriptionResponse, RecordsError> {
    let mut joined = String::new();
    for result in final_results {
        joined.push_str(&result.transcript);
    }
    let mut extra = Object::new();
    extra.insert("task".to_owned(), Value::from(TRANSCRIBE_TASK));
    if response_format == VERBOSE_JSON {
        let mut words = Vec::new();
        if timestamp_granularities
            .is_some_and(|names| names.iter().any(|name| name == WORD_GRANULARITY))
        {
            for result in final_results {
                for word in &result.words {
                    words.push(word_value(word)?);
                }
            }
        }
        if !words.is_empty() {
            extra.insert("words".to_owned(), Value::Array(words));
        }
        if let Some(duration) = duration_seconds {
            extra.insert("duration".to_owned(), Value::from(duration));
        }
    }
    Ok(TranscriptionResponse {
        text: Some(joined.trim().to_owned()),
        extra,
    })
}

fn word_value(word: &WordTiming) -> Result<Value, RecordsError> {
    let mut entry = Object::new();
    entry.insert("word".to_owned(), Value::from(word.word.as_str()));
    entry.insert(
        "start".to_owned(),
        Value::from(seconds(word.start_time_ms)?),
    );
    entry.insert("end".to_owned(), Value::from(seconds(word.end_time_ms)?));
    Ok(Value::Object(entry))
}

fn seconds(milliseconds: i64) -> Result<f64, RecordsError> {
    // the source divides by 1000.0; Decimal keeps the intermediate exact
    let seconds = Decimal::from(milliseconds)
        .checked_div(Decimal::from(1_000_u32))
        .and_then(|value| value.to_f64())
        .ok_or(RecordsError::Timing { field: "words" })?;
    Ok(seconds)
}
