use thiserror::Error;

use crate::benchmark_aggregates::row::SessionAggRow;

pub const MAX_ROUTER_READ_BYTES: usize = 1_048_576;

#[derive(Debug, Error)]
pub enum DecodeError {
    #[error("the router read payload has {bytes} bytes, past the {limit}-byte limit")]
    TooLarge { bytes: usize, limit: usize },
    #[error("the router read payload is not a list of session aggregate rows: {source}")]
    Invalid { source: serde_json::Error },
}

// Bound first, decode second: the payload never reaches serde past the limit
pub fn decode_session_agg_rows(bytes: &[u8]) -> Result<Vec<SessionAggRow>, DecodeError> {
    if bytes.len() > MAX_ROUTER_READ_BYTES {
        return Err(DecodeError::TooLarge {
            bytes: bytes.len(),
            limit: MAX_ROUTER_READ_BYTES,
        });
    }
    serde_json::from_slice(bytes).map_err(|source| DecodeError::Invalid { source })
}
