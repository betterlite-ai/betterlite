use betterlite_openai::api::audio_subtitles::{SRT_RESPONSE_FORMAT, VTT_RESPONSE_FORMAT};
use betterlite_openai::openai_base_config::Object;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde::{Deserialize, Serialize};
use serde_json::{from_value, to_value};

use super::error::RecordsError;
use super::tokens::{
    Milliseconds, SonioxToken, render_soniox_tokens, render_soniox_tokens_as_srt,
    render_soniox_tokens_as_vtt,
};

pub const TRANSCRIBE_TASK: &str = "transcribe";
pub const VERBOSE_JSON: &str = "verbose_json";
const WORDS_FIELD: &str = "words";
const HIDDEN_RAW: &str = "soniox_raw";

#[derive(Debug, Clone, Default)]
pub struct SonioxTranscriptPayload {
    pub transcription: Object,
    pub transcript: Object,
}

impl TryFrom<Object> for SonioxTranscriptPayload {
    type Error = RecordsError;

    fn try_from(payload: Object) -> Result<Self, Self::Error> {
        if !payload.contains_key("transcript") {
            return Ok(Self {
                transcription: Object::new(),
                transcript: payload,
            });
        }
        let transcription = object_at(&payload, "transcription")?;
        let transcript = object_at(&payload, "transcript")?;
        Ok(Self {
            transcription,
            transcript,
        })
    }
}

fn object_at(payload: &Object, field: &'static str) -> Result<Object, RecordsError> {
    let Some(value) = payload.get(field) else {
        return Ok(Object::new());
    };
    if value.is_null() {
        return Ok(Object::new());
    }
    from_value(value.clone()).map_err(|source| RecordsError::Value { field, source })
}

#[derive(Debug, Deserialize)]
struct TranscriptFields {
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    tokens: Option<Vec<SonioxToken>>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct TranscriptView {
    pub(super) text: Option<String>,
    pub(super) tokens: Vec<SonioxToken>,
}

impl TranscriptView {
    pub(super) fn from_transcript(transcript: &Object) -> Result<Self, RecordsError> {
        let encoded = to_value(transcript).map_err(|source| RecordsError::Value {
            field: "transcript",
            source,
        })?;
        let fields: TranscriptFields =
            from_value(encoded).map_err(|source| RecordsError::Value {
                field: "transcript",
                source,
            })?;
        Ok(Self {
            text: fields.text,
            tokens: fields.tokens.unwrap_or_default(),
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct SonioxTranscriptionResponse {
    pub text: Option<String>,
    pub usage: Option<Object>,
    #[serde(flatten)]
    pub extra: Object,
    #[serde(skip)]
    pub hidden_params: Object,
}

pub fn build_response_from_payload(
    payload: &SonioxTranscriptPayload,
    model_response: Option<SonioxTranscriptionResponse>,
    response_format: Option<&str>,
) -> Result<SonioxTranscriptionResponse, RecordsError> {
    let translated = TranscriptView::from_transcript(&payload.transcript)?;
    let mut response = model_response.unwrap_or_default();
    response.text = Some(response_text(&translated, response_format));
    response.extra.insert(
        "task".to_owned(),
        to_value(TRANSCRIBE_TASK).map_err(|source| RecordsError::Value {
            field: "task",
            source,
        })?,
    );
    set_duration(&mut response, &payload.transcription)?;
    set_language(&mut response, &translated.tokens)?;
    set_words(&mut response, &translated.tokens, response_format)?;
    set_hidden(&mut response, payload)?;
    Ok(response)
}

fn response_text(translated: &TranscriptView, response_format: Option<&str>) -> String {
    if response_format == Some(SRT_RESPONSE_FORMAT) && !translated.tokens.is_empty() {
        return render_soniox_tokens_as_srt(&translated.tokens);
    }
    if response_format == Some(VTT_RESPONSE_FORMAT) && !translated.tokens.is_empty() {
        return render_soniox_tokens_as_vtt(&translated.tokens);
    }
    plain_text(translated)
}

fn plain_text(translated: &TranscriptView) -> String {
    let tokens = &translated.tokens;
    let has_speaker = tokens.iter().any(|token| token.speaker.is_some());
    let has_language = tokens.iter().any(|token| token.language.is_some());
    if (has_speaker || has_language) && !tokens.is_empty() {
        return render_soniox_tokens(tokens);
    }
    if let Some(text) = translated.text.as_deref().filter(|text| !text.is_empty()) {
        return text.to_owned();
    }
    if !tokens.is_empty() {
        return tokens
            .iter()
            .filter_map(|token| token.text.as_deref())
            .collect();
    }
    String::new()
}

fn set_duration(
    response: &mut SonioxTranscriptionResponse,
    transcription: &Object,
) -> Result<(), RecordsError> {
    let Some(value) = transcription.get("audio_duration_ms") else {
        return Ok(());
    };
    if value.is_null() {
        return Ok(());
    }
    let Ok(milliseconds) = from_value::<Milliseconds>(value.clone()) else {
        return Ok(());
    };
    let Some(shown) = seconds_from_millis(milliseconds.value()) else {
        return Ok(());
    };
    let encoded = to_value(shown).map_err(|source| RecordsError::Value {
        field: "duration",
        source,
    })?;
    response.extra.insert("duration".to_owned(), encoded);
    Ok(())
}

fn set_language(
    response: &mut SonioxTranscriptionResponse,
    tokens: &[SonioxToken],
) -> Result<(), RecordsError> {
    let has_language = tokens.iter().any(|token| token.language.is_some());
    if !has_language {
        return Ok(());
    }
    let mut languages: Vec<&str> = Vec::new();
    for token in tokens {
        let language = token.language.as_deref().filter(|value| !value.is_empty());
        let Some(language) = language else {
            continue;
        };
        if !languages.contains(&language) {
            languages.push(language);
        }
    }
    let Some(language) = languages.first() else {
        return Ok(());
    };
    if languages.len() != 1 {
        return Ok(());
    }
    let encoded = to_value(language).map_err(|source| RecordsError::Value {
        field: "language",
        source,
    })?;
    response.extra.insert("language".to_owned(), encoded);
    Ok(())
}

#[derive(Debug, Serialize)]
struct WordEntry {
    word: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    start: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    end: Option<f64>,
}

fn set_words(
    response: &mut SonioxTranscriptionResponse,
    tokens: &[SonioxToken],
    response_format: Option<&str>,
) -> Result<(), RecordsError> {
    if response_format != Some(VERBOSE_JSON) || tokens.is_empty() {
        return Ok(());
    }
    let mut words: Vec<WordEntry> = Vec::new();
    for token in tokens {
        words.push(WordEntry {
            word: token.text.clone().unwrap_or_default(),
            start: word_seconds(token.start_ms)?,
            end: word_seconds(token.end_ms)?,
        });
    }
    let encoded = to_value(words).map_err(|source| RecordsError::Value {
        field: WORDS_FIELD,
        source,
    })?;
    response.extra.insert(WORDS_FIELD.to_owned(), encoded);
    Ok(())
}

fn word_seconds(timing: Option<Milliseconds>) -> Result<Option<f64>, RecordsError> {
    match timing {
        Some(timing) => Ok(Some(
            seconds_from_millis(timing.value())
                .ok_or(RecordsError::Timing { field: WORDS_FIELD })?,
        )),
        None => Ok(None),
    }
}

fn seconds_from_millis(milliseconds: i64) -> Option<f64> {
    // the source divides by 1000.0; Decimal keeps the intermediate exact
    Decimal::from(milliseconds)
        .checked_div(Decimal::from(1_000_u32))
        .and_then(|value| value.to_f64())
}

fn set_hidden(
    response: &mut SonioxTranscriptionResponse,
    payload: &SonioxTranscriptPayload,
) -> Result<(), RecordsError> {
    let mut raw = Object::new();
    raw.insert(
        "transcription".to_owned(),
        to_value(&payload.transcription).map_err(|source| RecordsError::Value {
            field: HIDDEN_RAW,
            source,
        })?,
    );
    raw.insert(
        "transcript".to_owned(),
        to_value(&payload.transcript).map_err(|source| RecordsError::Value {
            field: HIDDEN_RAW,
            source,
        })?,
    );
    let encoded = to_value(raw).map_err(|source| RecordsError::Value {
        field: HIDDEN_RAW,
        source,
    })?;
    response
        .hidden_params
        .insert(HIDDEN_RAW.to_owned(), encoded);
    Ok(())
}
