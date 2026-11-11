use crate::controls::json_value::JsonValue;

pub const MAX_CONTROL_INT_DIGITS: usize = 18;
pub const CONTROL_OPTIONS_KEY: &str = "_litellm_control";
const STREAM_CHUNK_SIZE_PARAM: &str = "stream_chunk_size";
const STREAM_CHUNK_SIZE_LIMIT: u64 = 1_000_000_000_000_000_000;

// Only a validated value exists: there is no Deserialize impl, so request JSON can never construct one.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ControlOptions {
    pub stream_chunk_size: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum InvalidControlOption {
    #[error("invalid stream_chunk_size: expected a positive integer of at most 18 digits")]
    StreamChunkSize,
}

impl InvalidControlOption {
    #[must_use]
    pub const fn param(&self) -> &'static str {
        match self {
            Self::StreamChunkSize => STREAM_CHUNK_SIZE_PARAM,
        }
    }
}

impl ControlOptions {
    pub const fn new(stream_chunk_size: Option<u64>) -> Result<Self, InvalidControlOption> {
        let Some(size) = stream_chunk_size else {
            return Ok(Self {
                stream_chunk_size: None,
            });
        };
        if size == 0 || size >= STREAM_CHUNK_SIZE_LIMIT {
            return Err(InvalidControlOption::StreamChunkSize);
        }
        Ok(Self {
            stream_chunk_size: Some(size),
        })
    }
}

// LiteLLM's strict integer field accepts a decimal string of at most 18 ASCII digits, then the same range check applies.
pub fn parse_control_options(
    stream_chunk_size: Option<&JsonValue>,
) -> Result<ControlOptions, InvalidControlOption> {
    let Some(value) = stream_chunk_size else {
        return ControlOptions::new(None);
    };
    match value {
        JsonValue::Null => ControlOptions::new(None),
        JsonValue::Number(number) => {
            let size = number
                .as_u64()
                .ok_or(InvalidControlOption::StreamChunkSize)?;
            ControlOptions::new(Some(size))
        }
        JsonValue::Text(text) => {
            let size = decimal_integer(text).ok_or(InvalidControlOption::StreamChunkSize)?;
            ControlOptions::new(Some(size))
        }
        JsonValue::Bool(_) | JsonValue::Array(_) | JsonValue::Object(_) => {
            Err(InvalidControlOption::StreamChunkSize)
        }
    }
}

fn decimal_integer(text: &str) -> Option<u64> {
    if text.is_empty() || text.len() > MAX_CONTROL_INT_DIGITS {
        return None;
    }
    if !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse::<u64>().ok()
}

mod tests;
