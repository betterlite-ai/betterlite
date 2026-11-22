use betterlite_openai::api::responses_shapes::wire::{
    ShapeDecodeError, WireDecodeLimits, decode_responses_shapes,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::api::api_boundaries::ApiLimits;

use super::record::{PollRecord, PolledResponse};

// the envelope is ours; a plain response object is a record written before owners existed
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StoredPollState {
    Record(PollRecord),
    Legacy(PolledResponse),
}

impl StoredPollState {
    #[must_use]
    pub const fn public(&self) -> &PolledResponse {
        match self {
            Self::Record(record) => &record.response,
            Self::Legacy(response) => response,
        }
    }
}

#[derive(Debug, Error)]
pub enum PollingDecodeError {
    #[error(transparent)]
    Shapes(#[from] ShapeDecodeError),
}

pub fn decode_responses_poll_state(
    bytes: &[u8],
    limits: ApiLimits,
) -> Result<StoredPollState, PollingDecodeError> {
    let wire = WireDecodeLimits {
        max_json_bytes: limits.max_json_bytes,
        max_event_bytes: limits.max_event_bytes,
        max_items: limits.max_items,
    };
    decode_responses_shapes(bytes, wire).map_err(PollingDecodeError::from)
}
