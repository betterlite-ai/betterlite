use betterlite_openai::openai_base_config::Object;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;
use serde_json::to_value;

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
pub fn normalize_language(language: Option<&str>) -> String {
    let Some(code) = language.filter(|code| !code.is_empty()) else {
        return DEFAULT_LANGUAGE_CODE.to_owned();
    };
    if code.contains('-') {
        return code.to_owned();
    }
    let lowered = code.to_lowercase();
    for (bare, bcp47) in BARE_LANGUAGE_CODES {
        if lowered == bare {
            return bcp47.to_owned();
        }
    }
    code.to_owned()
}

// OpenAI's language becomes language_code and word timings set the Riva flag; every other non-null input is kept as sent
pub fn map_openai_params(non_default_params: &Object) -> Result<Object, RecordsError> {
    let mut optional_params = Object::new();
    for (name, value) in non_default_params {
        match name.as_str() {
            "language" if !value.is_null() => {
                let language = to_value(normalize_language(value.as_str())).map_err(|source| {
                    RecordsError::Value {
                        field: "language_code",
                        source,
                    }
                })?;
                optional_params.insert("language_code".to_owned(), language);
            }
            "timestamp_granularities" => {
                apply_granularities(non_default_params, &mut optional_params)?;
            }
            _ if value.is_null() => {}
            _ => {
                optional_params.insert(name.clone(), value.clone());
            }
        }
    }
    Ok(optional_params)
}

fn apply_granularities(params: &Object, mapped: &mut Object) -> Result<(), RecordsError> {
    let Some(names) =
        granularities_at(params, "timestamp_granularities", "timestamp_granularities")?
    else {
        return Ok(());
    };
    if names.iter().any(|entry| entry == "word") {
        let enabled = to_value(true).map_err(|source| RecordsError::Value {
            field: "enable_word_time_offsets",
            source,
        })?;
        mapped.insert("enable_word_time_offsets".to_owned(), enabled);
    }
    let listed = to_value(names).map_err(|source| RecordsError::Value {
        field: "timestamp_granularities",
        source,
    })?;
    mapped.insert("timestamp_granularities".to_owned(), listed);
    Ok(())
}

type Optional<T> = Result<Option<T>, RecordsError>;

pub(super) fn text_option(params: &Object, field: &'static str) -> Optional<String> {
    let Some(value) = params.get(field) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let Some(text) = value.as_str() else {
        return Err(RecordsError::Text { field });
    };
    Ok(Some(text.to_owned()))
}

pub(super) fn bool_option(params: &Object, field: &'static str) -> Optional<bool> {
    let Some(value) = params.get(field) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    value
        .as_bool()
        .map(Some)
        .ok_or(RecordsError::Boolean { field })
}

pub(super) fn integer_option(params: &Object, field: &'static str) -> Optional<u32> {
    params
        .get(field)
        .filter(|value| !value.is_null())
        .map_or(Ok(None), |value| {
            integer_of(value.as_u64(), field).map(Some)
        })
}

pub(super) fn integer_at(values: &Object, name: &str, field: &'static str) -> Optional<u32> {
    let Some(value) = values.get(name) else {
        return Ok(None);
    };
    integer_of(value.as_u64(), field).map(Some)
}

pub(super) fn decimal_at(values: &Object, name: &str, field: &'static str) -> Optional<Decimal> {
    let Some(value) = values.get(name) else {
        return Ok(None);
    };
    let number = value
        .as_f64()
        .and_then(Decimal::from_f64)
        .ok_or(RecordsError::Number { field })?;
    Ok(Some(number))
}

fn integer_of(value: Option<u64>, field: &'static str) -> Result<u32, RecordsError> {
    let Some(number) = value else {
        return Err(RecordsError::Integer { field });
    };
    u32::try_from(number)
        .ok()
        .ok_or(RecordsError::Integer { field })
}

fn granularities_at(values: &Object, name: &str, field: &'static str) -> Optional<Vec<String>> {
    let Some(value) = values.get(name) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let Some(entries) = value.as_array() else {
        return Err(RecordsError::Granularities { field });
    };
    let mut names = Vec::with_capacity(entries.len());
    for entry in entries {
        let Some(text) = entry.as_str() else {
            return Err(RecordsError::Granularities { field });
        };
        names.push(text.to_owned());
    }
    Ok(Some(names))
}
