use betterlite_openai::api::transcription_contract::TranscriptionResponse;
use betterlite_openai::openai_base_config::Object;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde::{Deserialize, Serialize};
use serde_json::to_value;

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

#[derive(Debug, Serialize)]
struct WordEntry<'word> {
    word: &'word str,
    start: f64,
    end: f64,
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
    let task = to_value(TRANSCRIBE_TASK).map_err(|source| RecordsError::Value {
        field: "task",
        source,
    })?;
    extra.insert("task".to_owned(), task);
    if response_format == VERBOSE_JSON {
        let words = word_entries(final_results, timestamp_granularities)?;
        if !words.is_empty() {
            let listed = to_value(words).map_err(|source| RecordsError::Value {
                field: "words",
                source,
            })?;
            extra.insert("words".to_owned(), listed);
        }
        if let Some(duration) = duration_seconds {
            let shown = to_value(duration).map_err(|source| RecordsError::Value {
                field: "duration",
                source,
            })?;
            extra.insert("duration".to_owned(), shown);
        }
    }
    Ok(TranscriptionResponse {
        text: Some(joined.trim().to_owned()),
        extra,
    })
}

fn word_entries<'word>(
    final_results: &'word [FinalResult],
    timestamp_granularities: Option<&[String]>,
) -> Result<Vec<WordEntry<'word>>, RecordsError> {
    let requested = timestamp_granularities
        .is_some_and(|names| names.iter().any(|name| name == WORD_GRANULARITY));
    if !requested {
        return Ok(Vec::new());
    }
    let mut entries = Vec::new();
    for result in final_results {
        for word in &result.words {
            entries.push(WordEntry {
                word: &word.word,
                start: seconds(word.start_time_ms)?,
                end: seconds(word.end_time_ms)?,
            });
        }
    }
    Ok(entries)
}

fn seconds(milliseconds: i64) -> Result<f64, RecordsError> {
    // the source divides by 1000.0; Decimal keeps the intermediate exact
    let converted = Decimal::from(milliseconds)
        .checked_div(Decimal::from(1_000_u32))
        .and_then(|value| value.to_f64())
        .ok_or(RecordsError::Timing { field: "words" })?;
    Ok(converted)
}
