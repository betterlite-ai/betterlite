mod text;

use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::Object;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;

use super::error::PrepareError;
use crate::api::soniox::records::{
    SONIOX_DEFAULT_ATTEMPTS, SONIOX_DEFAULT_CLEANUP, SONIOX_DEFAULT_POLL_INTERVAL,
    SONIOX_MAX_ATTEMPTS, SONIOX_MAX_POLL_INTERVAL, SONIOX_MIN_POLL_INTERVAL,
};

pub const POLL_INTERVAL_FIELD: &str = "soniox_polling_interval";
pub const MAX_ATTEMPTS_FIELD: &str = "soniox_max_polling_attempts";
pub const CLEANUP_FIELD: &str = "soniox_cleanup";
pub const FILENAME_FIELD: &str = "filename";
pub const AUDIO_URL_FIELD: &str = "audio_url";
pub const FILE_ID_FIELD: &str = "file_id";
pub const RESPONSE_FORMAT_FIELD: &str = "response_format";
pub const LANGUAGE_FIELD: &str = "language";

const MIN_ATTEMPTS: i64 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandlerOptions {
    pub poll_interval: Decimal,
    pub max_attempts: i64,
    pub cleanup: Vec<String>,
    pub filename_override: Option<String>,
    pub audio_url: Option<String>,
    pub file_id: Option<String>,
    pub response_format: Option<String>,
}

pub fn extract_options(params: &mut Object) -> Result<HandlerOptions, PrepareError> {
    let poll_interval = poll_interval_at(params)?;
    let max_attempts = max_attempts_at(params)?;
    let cleanup = cleanup_at(params)?;
    let filename_override = text_at(params, FILENAME_FIELD)?;
    let audio_url = text_at(params, AUDIO_URL_FIELD)?;
    let file_id = text_at(params, FILE_ID_FIELD)?;
    let response_format = text_at(params, RESPONSE_FORMAT_FIELD)?;
    drop(params.remove(LANGUAGE_FIELD));
    Ok(HandlerOptions {
        poll_interval,
        max_attempts,
        cleanup,
        filename_override,
        audio_url,
        file_id,
        response_format,
    })
}

fn poll_interval_at(params: &mut Object) -> Result<Decimal, PrepareError> {
    let Some(value) = params.remove(POLL_INTERVAL_FIELD) else {
        return bounded_interval(SONIOX_DEFAULT_POLL_INTERVAL);
    };
    let seconds = if let Some(text) = value.as_str() {
        text::seconds(text)
    } else {
        number_seconds(&value)?
    };
    let Some(parsed) = seconds else {
        return Err(PrepareError::Interval {
            field: POLL_INTERVAL_FIELD,
        });
    };
    bounded_interval(parsed)
}

fn bounded_interval(seconds: f64) -> Result<Decimal, PrepareError> {
    let bounded = if seconds.is_finite() {
        seconds.clamp(SONIOX_MIN_POLL_INTERVAL, SONIOX_MAX_POLL_INTERVAL)
    } else {
        SONIOX_DEFAULT_POLL_INTERVAL
    };
    Decimal::from_f64(bounded).ok_or(PrepareError::Interval {
        field: POLL_INTERVAL_FIELD,
    })
}

fn number_seconds(value: &JsonValue) -> Result<Option<f64>, PrepareError> {
    let literal = value.to_string();
    let seconds = value.as_f64();
    if !literal.contains(['.', 'e', 'E']) && seconds.is_some_and(|number| !number.is_finite()) {
        return Err(PrepareError::Interval {
            field: POLL_INTERVAL_FIELD,
        });
    }
    Ok(seconds)
}

fn max_attempts_at(params: &mut Object) -> Result<i64, PrepareError> {
    let Some(value) = params.remove(MAX_ATTEMPTS_FIELD) else {
        return Ok(SONIOX_DEFAULT_ATTEMPTS);
    };
    if value.is_null() || value.is_boolean() || value.is_array() || value.is_object() {
        return Err(PrepareError::Attempts {
            field: MAX_ATTEMPTS_FIELD,
        });
    }
    let attempts = if let Some(text) = value.as_str() {
        text::attempts(text)
    } else {
        number_attempts(&value)
    };
    let Some(parsed) = attempts else {
        return Ok(SONIOX_DEFAULT_ATTEMPTS);
    };
    Ok(parsed.clamp(MIN_ATTEMPTS, SONIOX_MAX_ATTEMPTS))
}

fn number_attempts(value: &JsonValue) -> Option<i64> {
    let literal = value.to_string();
    if !literal.contains(['.', 'e', 'E']) {
        return Some(text::saturated_integer(&literal));
    }
    let number = value.as_f64().filter(|number| number.is_finite())?;
    text::truncated_attempts(number)
}

fn cleanup_at(params: &mut Object) -> Result<Vec<String>, PrepareError> {
    let Some(value) = params.remove(CLEANUP_FIELD) else {
        return Ok(SONIOX_DEFAULT_CLEANUP.map(str::to_owned).to_vec());
    };
    if value.is_null() {
        return Ok(Vec::new());
    }
    if let Some(text) = value.as_str() {
        return Ok(vec![text.to_owned()]);
    }
    let Some(entries) = value.as_array() else {
        return Err(PrepareError::Cleanup {
            field: CLEANUP_FIELD,
        });
    };
    entries.iter().map(name_entry).collect()
}

fn name_entry(value: &JsonValue) -> Result<String, PrepareError> {
    let Some(text) = value.as_str() else {
        return Err(PrepareError::Cleanup {
            field: CLEANUP_FIELD,
        });
    };
    Ok(text.to_owned())
}

fn text_at(params: &mut Object, field: &'static str) -> Result<Option<String>, PrepareError> {
    let Some(value) = params.remove(field) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let Some(text) = value.as_str() else {
        return Err(PrepareError::Text { field });
    };
    Ok(Some(text.to_owned()))
}
