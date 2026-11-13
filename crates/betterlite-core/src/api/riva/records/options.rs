use betterlite_openai::openai_base_config::Object;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;
use serde_json::{Number, Value};

use super::error::RecordsError;

pub const DEFAULT_LANGUAGE_CODE: &str = "en-US";
pub const DEFAULT_SAMPLE_RATE_HERTZ: u32 = 16_000;
pub const DEFAULT_ENCODING: &str = "LINEAR_PCM";
pub const DEFAULT_AUDIO_CHANNEL_COUNT: u32 = 1;
pub const DEFAULT_MAX_ALTERNATIVES: u32 = 1;
pub const DEFAULT_RESPONSE_FORMAT: &str = "json";

const BARE_LANGUAGE_CODES: [(&str, &str); 12] = [
    ("en", "en-US"),
    ("es", "es-ES"),
    ("de", "de-DE"),
    ("fr", "fr-FR"),
    ("it", "it-IT"),
    ("pt", "pt-BR"),
    ("ja", "ja-JP"),
    ("ko", "ko-KR"),
    ("zh", "zh-CN"),
    ("ru", "ru-RU"),
    ("hi", "hi-IN"),
    ("ar", "ar-SA"),
];

#[must_use]
pub fn normalize_language(language: &Value) -> String {
    let Value::String(language) = language else {
        return DEFAULT_LANGUAGE_CODE.to_owned();
    };
    if language.is_empty() {
        return DEFAULT_LANGUAGE_CODE.to_owned();
    }
    if language.contains('-') {
        return language.clone();
    }
    let lowered = language.to_lowercase();
    for (bare, bcp47) in BARE_LANGUAGE_CODES {
        if lowered == bare {
            return bcp47.to_owned();
        }
    }
    language.clone()
}

// OpenAI's language becomes language_code and word timings set the Riva flag; every other non-null input is kept as sent
pub fn map_openai_params(non_default_params: &Object) -> Result<Object, RecordsError> {
    let mut optional_params = Object::new();
    for (name, value) in non_default_params {
        if value.is_null() {
            continue;
        }
        if name == "language" {
            optional_params.insert(
                "language_code".to_owned(),
                Value::from(normalize_language(value)),
            );
            continue;
        }
        if name == "timestamp_granularities" {
            let names = granularities(value, "timestamp_granularities")?;
            if names.iter().any(|entry| entry == "word") {
                optional_params.insert("enable_word_time_offsets".to_owned(), Value::Bool(true));
            }
            optional_params.insert(
                "timestamp_granularities".to_owned(),
                Value::Array(
                    names
                        .iter()
                        .map(|name| Value::from(name.as_str()))
                        .collect(),
                ),
            );
            continue;
        }
        optional_params.insert(name.clone(), value.clone());
    }
    Ok(optional_params)
}

pub(super) fn text_option(
    params: &Object,
    field: &'static str,
) -> Result<Option<String>, RecordsError> {
    match params.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(RecordsError::Text { field }),
    }
}

pub(super) fn bool_option(
    params: &Object,
    field: &'static str,
) -> Result<Option<bool>, RecordsError> {
    match params.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(flag)) => Ok(Some(*flag)),
        Some(_) => Err(RecordsError::Boolean { field }),
    }
}

pub(super) fn integer_option(
    params: &Object,
    field: &'static str,
) -> Result<Option<u32>, RecordsError> {
    match params.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(number)) => integer_number(number, field).map(Some),
        Some(_) => Err(RecordsError::Integer { field }),
    }
}

pub(super) fn granularity_option(
    params: &Object,
    field: &'static str,
) -> Result<Option<Vec<String>>, RecordsError> {
    match params.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => granularities(value, field).map(Some),
    }
}

pub(super) fn granularities(
    value: &Value,
    field: &'static str,
) -> Result<Vec<String>, RecordsError> {
    let Value::Array(entries) = value else {
        return Err(RecordsError::Granularities { field });
    };
    let mut names = Vec::with_capacity(entries.len());
    for entry in entries {
        let Value::String(name) = entry else {
            return Err(RecordsError::Granularities { field });
        };
        names.push(name.clone());
    }
    Ok(names)
}

pub(super) fn integer_value(value: &Value, field: &'static str) -> Result<u32, RecordsError> {
    let Value::Number(number) = value else {
        return Err(RecordsError::Integer { field });
    };
    integer_number(number, field)
}

fn integer_number(number: &Number, field: &'static str) -> Result<u32, RecordsError> {
    let Some(value) = number.as_u64() else {
        return Err(RecordsError::Integer { field });
    };
    u32::try_from(value)
        .ok()
        .ok_or(RecordsError::Integer { field })
}

pub(super) fn decimal_value(value: &Value, field: &'static str) -> Result<Decimal, RecordsError> {
    let Value::Number(number) = value else {
        return Err(RecordsError::Number { field });
    };
    number
        .as_f64()
        .and_then(Decimal::from_f64)
        .ok_or(RecordsError::Number { field })
}
