use betterlite_models::decisions::Json;
use thiserror::Error;

use crate::api::api_boundaries::{ApiBoundariesInput, ApiLimitError, ApiLimits, EventBoundary};

#[derive(Debug, Error)]
pub enum StreamDecodeError {
    #[error(transparent)]
    Limit(#[from] ApiLimitError),
    #[error("the stream payload is not JSON: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
}

// json.loads for both loaders: the value comes back in whatever shape the payload had
pub fn decode_responses_stream_value(
    bytes: &[u8],
    limits: &ApiLimits,
) -> Result<Json, StreamDecodeError> {
    limits.check(ApiBoundariesInput::Event(EventBoundary {
        known_bytes: bytes.len(),
    }))?;
    serde_json::from_slice::<Json>(bytes).map_err(|source| StreamDecodeError::Json { source })
}

pub trait JsonShape {
    fn is_json_object(&self) -> bool;
    fn is_json_array(&self) -> bool;
    fn optional_str(&self) -> Option<&str>;
    fn json_array_or_empty(&self) -> &[Json];
    fn is_str_mapping(&self) -> bool;
}

impl JsonShape for Json {
    fn is_json_object(&self) -> bool {
        matches!(self, Self::Object(_))
    }

    fn is_json_array(&self) -> bool {
        matches!(self, Self::List(_))
    }

    fn optional_str(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text),
            Self::Null | Self::Bool(_) | Self::Number(_) | Self::List(_) | Self::Object(_) => None,
        }
    }

    fn json_array_or_empty(&self) -> &[Json] {
        match self {
            Self::List(items) => items,
            Self::Null | Self::Bool(_) | Self::Number(_) | Self::Text(_) | Self::Object(_) => &[],
        }
    }

    fn is_str_mapping(&self) -> bool {
        match self {
            Self::Object(fields) => fields.values().all(|item| matches!(item, Self::Text(_))),
            Self::Null | Self::Bool(_) | Self::Number(_) | Self::Text(_) | Self::List(_) => false,
        }
    }
}
