use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::decisions::Json;
use betterlite_openai::api::transcription_contract::TranscriptionResponse;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::transcription::duration::duration_number;
use indexmap::IndexMap;
use serde_json::{Deserializer, Number};

use crate::gemini_response::decode::MAX_RESPONSE_BYTES;
use crate::studio_transcription::error::StudioTranscriptionError;
use crate::studio_transcription::types::{Content, InteractionResponse, Usage, WordAnnotation};
use crate::vertex_recognition::response::parse_duration_seconds;

const COMPLETED_STATUS: &str = "completed";
const TEXT_CONTENT_TYPE: &str = "text";
const WORD_INFO_ANNOTATION_TYPE: &str = "word_info";
const AUDIO_MODALITY: &str = "audio";
const TRANSCRIBE_TASK: &str = "transcribe";

pub fn decode_studio_transcription(
    reply: &UpstreamResponse,
) -> Result<TranscriptionResponse, StudioTranscriptionError> {
    if reply.body.len() > MAX_RESPONSE_BYTES {
        return Err(StudioTranscriptionError::TooLarge {
            size: reply.body.len(),
        });
    }
    let mut decoder = Deserializer::from_slice(&reply.body);
    let parsed: InteractionResponse = serde_path_to_error::deserialize(&mut decoder)
        .map_err(|error| decode_error(reply.status, error))?;
    decoder
        .end()
        .map_err(|source| StudioTranscriptionError::NotJson {
            status: reply.status,
            source,
        })?;
    if parsed.status.as_deref() != Some(COMPLETED_STATUS) {
        return Err(StudioTranscriptionError::Incomplete {
            status: parsed.status,
            upstream: reply.status,
        });
    }
    project(parsed)
}

// a syntax failure is the source's non-JSON reply, while a typed field that fails is pydantic's ValidationError
fn decode_error(
    status: u16,
    error: serde_path_to_error::Error<serde_json::Error>,
) -> StudioTranscriptionError {
    let path = error.path().to_string();
    let source = error.into_inner();
    if source.is_data() {
        return StudioTranscriptionError::Shape { path, source };
    }
    StudioTranscriptionError::NotJson { status, source }
}

fn word_record(annotation: &WordAnnotation) -> Option<Json> {
    if annotation.kind.as_deref() != Some(WORD_INFO_ANNOTATION_TYPE) {
        return None;
    }
    let text = annotation.text.as_ref()?;
    let mut entries = IndexMap::new();
    entries.insert("word".to_owned(), Json::Text(text.clone()));
    let start = parse_duration_seconds(annotation.start_offset.as_deref())
        .and_then(|seconds| duration_number(Some(seconds)).ok());
    if let Some(started) = start {
        entries.insert("start".to_owned(), started);
    }
    let end = parse_duration_seconds(annotation.end_offset.as_deref())
        .and_then(|seconds| duration_number(Some(seconds)).ok());
    if let Some(ended) = end {
        entries.insert("end".to_owned(), ended);
    }
    if let Some(speaker) = &annotation.speaker {
        entries.insert("speaker".to_owned(), Json::Text(speaker.clone()));
    }
    Some(Json::Object(entries))
}

fn usage_record(usage: &Usage) -> Result<Json, StudioTranscriptionError> {
    let audio_tokens = usage
        .input_tokens_by_modality
        .iter()
        .filter(|entry| entry.modality.as_deref() == Some(AUDIO_MODALITY))
        .try_fold(0_i64, |total, entry| total.checked_add(entry.tokens))
        .ok_or(StudioTranscriptionError::UsageOverflow)?;
    let text_tokens = usage
        .total_input_tokens
        .checked_sub(audio_tokens)
        .ok_or(StudioTranscriptionError::UsageOverflow)?;
    let token_count = |value: i64| Json::Number(Number::from(value));
    let total_input = usage.total_input_tokens;
    let total_output = usage.total_output_tokens;
    let total = usage.total_tokens;
    let mut details = IndexMap::new();
    details.insert("audio_tokens".to_owned(), token_count(audio_tokens));
    details.insert("text_tokens".to_owned(), token_count(text_tokens));
    let mut record = IndexMap::new();
    record.insert("type".to_owned(), Json::Text("tokens".to_owned()));
    record.insert("input_tokens".to_owned(), token_count(total_input));
    record.insert("output_tokens".to_owned(), token_count(total_output));
    record.insert("total_tokens".to_owned(), token_count(total));
    record.insert("input_token_details".to_owned(), Json::Object(details));
    Ok(Json::Object(record))
}

fn last_word_end(word: &Json) -> Option<Json> {
    if let Json::Object(entries) = word {
        return entries.get("end").cloned();
    }
    None
}

fn project(parsed: InteractionResponse) -> Result<TranscriptionResponse, StudioTranscriptionError> {
    let text_contents: Vec<&Content> = parsed
        .steps
        .iter()
        .flat_map(|step| step.content.iter())
        .filter(|content| {
            content.kind.as_deref() == Some(TEXT_CONTENT_TYPE) && content.text.is_some()
        })
        .collect();
    let text = text_contents
        .iter()
        .filter_map(|content| content.text.as_deref())
        .collect::<Vec<_>>()
        .join(" ");
    let words: Vec<Json> = text_contents
        .iter()
        .flat_map(|content| content.annotations.iter())
        .filter_map(word_record)
        .collect();
    let mut extra = IndexMap::new();
    extra.insert("task".to_owned(), Json::Text(TRANSCRIBE_TASK.to_owned()));
    if let Some(end) = words.last().and_then(last_word_end) {
        extra.insert("duration".to_owned(), end);
    }
    if !words.is_empty() {
        extra.insert("words".to_owned(), Json::List(words));
    }
    if let Some(usage) = parsed.usage {
        extra.insert("usage".to_owned(), usage_record(&usage)?);
    }
    let value = serde_json::to_value(Json::Object(extra))
        .map_err(|source| StudioTranscriptionError::Json { source })?;
    let object: Object = serde_json::from_value(value)
        .map_err(|source| StudioTranscriptionError::Json { source })?;
    Ok(TranscriptionResponse {
        text: Some(text),
        extra: object,
    })
}
