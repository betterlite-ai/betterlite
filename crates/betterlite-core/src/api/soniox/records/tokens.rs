use std::ops::Div as _;

use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::api::audio_subtitles::{Speaker, SubtitleToken};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde::Deserialize;
use serde_json::Number;

use super::error::RecordsError;

pub const TRANSLATION_STATUS: &str = "translation";

#[derive(Debug, Clone, Copy)]
pub(super) enum FloatMilliseconds {
    Exact(Decimal),
    Approximate(f64),
    NotFinite,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum SpeakerValue {
    Flag(bool),
    Number(i64),
    Text(String),
}

impl SpeakerValue {
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Flag(flag) => {
                if *flag {
                    "True".to_owned()
                } else {
                    "False".to_owned()
                }
            }
            Self::Number(number) => number.to_string(),
            Self::Text(text) => text.clone(),
        }
    }

    #[must_use]
    pub fn subtitle(&self) -> Speaker {
        match self {
            Self::Flag(flag) => Speaker::Number(i64::from(*flag)),
            Self::Number(number) => Speaker::Number(*number),
            Self::Text(text) => Speaker::Text(text.clone()),
        }
    }
}

impl PartialEq for SpeakerValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Flag(left), Self::Flag(right)) => left == right,
            (Self::Flag(flag), Self::Number(number)) | (Self::Number(number), Self::Flag(flag)) => {
                i64::from(*flag) == *number
            }
            (Self::Number(left), Self::Number(right)) => left == right,
            (Self::Text(left), Self::Text(right)) => left == right,
            (Self::Flag(_) | Self::Number(_), Self::Text(_))
            | (Self::Text(_), Self::Flag(_) | Self::Number(_)) => false,
        }
    }
}

impl Eq for SpeakerValue {}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SonioxToken {
    // the outer Option is a missing key, the inner one a null that a word entry writes as null
    #[serde(
        default,
        deserialize_with = "serde_with::rust::double_option::deserialize"
    )]
    pub text: Option<Option<JsonValue>>,
    #[serde(default)]
    pub start_ms: Option<JsonValue>,
    #[serde(default)]
    pub end_ms: Option<JsonValue>,
    #[serde(default)]
    pub speaker: Option<SpeakerValue>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub translation_status: Option<String>,
}

impl SonioxToken {
    #[must_use]
    pub fn text_string(&self) -> String {
        match self.text.as_ref() {
            Some(Some(JsonValue::Text(text))) => text.clone(),
            _ => String::new(),
        }
    }

    #[must_use]
    pub fn text_value(&self) -> JsonValue {
        match self.text.as_ref() {
            Some(Some(value)) => value.clone(),
            Some(None) => JsonValue::Null,
            None => JsonValue::Text(String::new()),
        }
    }

    pub fn joined_text(&self) -> Result<&str, RecordsError> {
        match self.text.as_ref() {
            None => Ok(""),
            Some(Some(JsonValue::Text(text))) => Ok(text.as_str()),
            Some(_) => Err(RecordsError::Text {
                field: "transcript",
            }),
        }
    }
}

#[must_use]
pub fn render_soniox_tokens(tokens: &[SonioxToken]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut current_speaker: Option<&SpeakerValue> = None;
    let mut current_language: Option<&str> = None;
    for token in tokens {
        let mut text = token.text_string();
        let speaker = token
            .speaker
            .as_ref()
            .filter(|speaker| current_speaker != Some(*speaker));
        if let Some(speaker) = speaker {
            if current_speaker.is_some() {
                parts.push(String::from("\n\n"));
            }
            current_speaker = Some(speaker);
            current_language = None;
            parts.push(format!("Speaker {}:", speaker.label()));
        }
        let language = token
            .language
            .as_deref()
            .filter(|language| current_language != Some(*language));
        if let Some(language) = language {
            current_language = Some(language);
            let prefix = if token.translation_status.as_deref() == Some(TRANSLATION_STATUS) {
                "[Translation] "
            } else {
                ""
            };
            parts.push(format!("\n{prefix}[{language}] "));
            text = text
                .trim_start_matches(is_whitespace_or_separator)
                .to_owned();
        }
        parts.push(text);
    }
    parts.concat()
}

fn is_whitespace_or_separator(character: char) -> bool {
    if character.is_whitespace() {
        return true;
    }
    matches!(character, '\u{1c}'..='\u{1f}')
}

pub(super) fn subtitle_tokens(tokens: &[SonioxToken]) -> Vec<SubtitleToken> {
    let mut kept: Vec<SubtitleToken> = Vec::new();
    for token in tokens {
        if token.translation_status.as_deref() == Some(TRANSLATION_STATUS) {
            continue;
        }
        kept.push(SubtitleToken {
            text: token.text_string(),
            start_ms: subtitle_milliseconds(token.start_ms.as_ref()),
            end_ms: subtitle_milliseconds(token.end_ms.as_ref()),
            speaker: token.speaker.as_ref().map(SpeakerValue::subtitle),
        });
    }
    kept
}

// the source keeps a subtitle timing only when it is a Python int, and a bool is one
fn subtitle_milliseconds(value: Option<&JsonValue>) -> Option<i64> {
    match value? {
        JsonValue::Bool(flag) => Some(i64::from(*flag)),
        JsonValue::Number(number) => number.as_i64(),
        JsonValue::Null | JsonValue::Text(_) | JsonValue::Array(_) | JsonValue::Object(_) => None,
    }
}

#[must_use]
pub(super) fn float_milliseconds(value: &JsonValue) -> Option<FloatMilliseconds> {
    match value {
        JsonValue::Bool(flag) => Some(FloatMilliseconds::Exact(Decimal::from(i64::from(*flag)))),
        JsonValue::Number(number) => milliseconds_from_text(&number.to_string()),
        JsonValue::Text(text) => milliseconds_from_text(text),
        JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => None,
    }
}

#[must_use]
pub(super) fn seconds_value(quantity: FloatMilliseconds) -> Option<JsonValue> {
    let seconds = match quantity {
        FloatMilliseconds::Exact(milliseconds) => {
            milliseconds.checked_div(Decimal::ONE_THOUSAND)?.to_f64()?
        }
        FloatMilliseconds::Approximate(milliseconds) => milliseconds.div(1_000.0_f64),
        FloatMilliseconds::NotFinite => return Some(JsonValue::Null),
    };
    Number::from_f64(seconds).map(JsonValue::Number)
}

fn milliseconds_from_text(text: &str) -> Option<FloatMilliseconds> {
    let trimmed = text.trim_matches(is_whitespace_or_separator);
    if !underscores_join_digits(trimmed) {
        return None;
    }
    let cleaned = trimmed.replace('_', "");
    let parsed = cleaned.parse::<f64>().ok()?;
    if !parsed.is_finite() {
        return Some(FloatMilliseconds::NotFinite);
    }
    let exact = cleaned
        .parse::<Decimal>()
        .ok()
        .filter(|number| number.checked_div(Decimal::ONE_THOUSAND).is_some());
    Some(match exact {
        Some(number) => FloatMilliseconds::Exact(number),
        None => FloatMilliseconds::Approximate(parsed),
    })
}

// float() takes an underscore only between two digits
fn underscores_join_digits(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.iter().enumerate().all(|(index, byte)| {
        if *byte != b'_' {
            return true;
        }
        let before = index.checked_sub(1).and_then(|before| bytes.get(before));
        let after = bytes.get(index + 1);
        before.is_some_and(u8::is_ascii_digit) && after.is_some_and(u8::is_ascii_digit)
    })
}
