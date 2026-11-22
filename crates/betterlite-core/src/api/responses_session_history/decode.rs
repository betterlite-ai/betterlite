use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use thiserror::Error;

use crate::api::api_boundaries::{ApiBoundariesInput, ApiLimitError, ApiLimits, JsonBoundary};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionLogJsonKind {
    Request,
    Metadata,
}

impl SessionLogJsonKind {
    // the fuzzer's first byte picks the column, so every byte value reaches one of the two
    #[must_use]
    pub const fn from_selector(selector: u8) -> Self {
        if selector.is_multiple_of(2) {
            Self::Request
        } else {
            Self::Metadata
        }
    }
}

#[derive(Debug, Error)]
pub enum SessionLogJsonError {
    #[error(transparent)]
    TooLarge(#[from] ApiLimitError),
    #[error("the stored {kind:?} column is not JSON")]
    Syntax {
        kind: SessionLogJsonKind,
        #[source]
        source: serde_json::Error,
    },
    #[error("the stored {kind:?} column is not a JSON object")]
    Shape { kind: SessionLogJsonKind },
}

// None is a stored request of JSON null, which LiteLLM treats as a payload cold storage must restore
pub fn decode_session_log_json(
    bytes: &[u8],
    kind: SessionLogJsonKind,
    limits: ApiLimits,
) -> Result<Option<IndexMap<String, Json>>, SessionLogJsonError> {
    limits.check(ApiBoundariesInput::Json(JsonBoundary {
        known_bytes: bytes.len(),
    }))?;
    let decoded = serde_json::from_slice(bytes)
        .map_err(|source| SessionLogJsonError::Syntax { kind, source })?;
    match (decoded, kind) {
        (Json::Object(fields), SessionLogJsonKind::Request | SessionLogJsonKind::Metadata) => {
            Ok(Some(fields))
        }
        (Json::Null, SessionLogJsonKind::Request) => Ok(None),
        (
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_),
            SessionLogJsonKind::Request | SessionLogJsonKind::Metadata,
        ) => Err(SessionLogJsonError::Shape { kind }),
    }
}
