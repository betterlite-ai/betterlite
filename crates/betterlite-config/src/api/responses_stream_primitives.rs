use crate::loader::EnvironmentLookup;
use thiserror::Error;

mod tests;

pub const STREAM_DURATION_ENV: &str = "LITELLM_MAX_STREAMING_DURATION_SECONDS";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum StreamDurationError {
    #[error("invalid number in LITELLM_MAX_STREAMING_DURATION_SECONDS")]
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct StreamDurationSettings {
    pub max_streaming_duration_seconds: Option<f64>,
}

// constants.py reads the float at import time and keeps no timeout when the variable is absent
pub fn loads_stream_duration_settings(
    environment: &dyn EnvironmentLookup,
) -> Result<StreamDurationSettings, StreamDurationError> {
    let Some(raw) = environment.lookup(STREAM_DURATION_ENV) else {
        return Ok(StreamDurationSettings::default());
    };
    let text = raw.trim();
    if text.is_empty() {
        return Ok(StreamDurationSettings::default());
    }
    let seconds = text
        .parse::<f64>()
        .map_err(|_source| StreamDurationError::Invalid)?;
    Ok(StreamDurationSettings {
        max_streaming_duration_seconds: Some(seconds),
    })
}
