use std::str::{Utf8Error, from_utf8};

use serde::de::DeserializeOwned;
use serde_json::Error as JsonError;

use crate::budget_permission_rows::JsonData;
use crate::managed_resource_rows::{ManagedRowsDecodeError, check_depth};

const MAX_BATCH_BYTES: usize = 0x0010_0000;
const MAX_BATCH_DEPTH: usize = 64;

#[derive(Debug, thiserror::Error)]
pub enum ManagedBatchRecordError {
    #[error("stored batch record is {len} bytes, over the {MAX_BATCH_BYTES} byte limit")]
    TooLarge { len: usize },
    #[error("stored batch record is not valid UTF-8")]
    InvalidUtf8(#[source] Utf8Error),
    #[error("stored batch record exceeds JSON depth {MAX_BATCH_DEPTH}")]
    TooDeep {
        #[source]
        source: ManagedRowsDecodeError,
    },
    #[error("stored batch record is not valid JSON")]
    Json(#[source] JsonError),
}

// B1035 owns the batch contract; the caller names it, since betterlite-openai sits above this crate
pub fn decode_managed_batch_record<Batch>(
    bytes: &[u8],
) -> Result<Option<Batch>, ManagedBatchRecordError>
where
    Batch: DeserializeOwned,
{
    if bytes.len() > MAX_BATCH_BYTES {
        return Err(ManagedBatchRecordError::TooLarge { len: bytes.len() });
    }
    let text = from_utf8(bytes).map_err(ManagedBatchRecordError::InvalidUtf8)?;
    check_depth(text).map_err(|source| ManagedBatchRecordError::TooDeep { source })?;
    let stored: JsonData = serde_json::from_str(text).map_err(ManagedBatchRecordError::Json)?;
    if is_falsey(&stored) {
        return Ok(None);
    }
    match stored {
        JsonData::String(inner) => {
            check_depth(&inner).map_err(|source| ManagedBatchRecordError::TooDeep { source })?;
            serde_json::from_str(&inner)
                .map(Some)
                .map_err(ManagedBatchRecordError::Json)
        }
        other => serde_json::to_value(other)
            .and_then(serde_json::from_value::<Batch>)
            .map(Some)
            .map_err(ManagedBatchRecordError::Json),
    }
}

fn is_falsey(value: &JsonData) -> bool {
    match value {
        JsonData::Null | JsonData::Bool(false) => true,
        JsonData::Number(number) => number
            .as_f64()
            .is_some_and(|float| float.to_bits() << 1 == 0),
        JsonData::String(text) => text.is_empty(),
        JsonData::Array(items) => items.is_empty(),
        JsonData::Object(fields) => fields.is_empty(),
        JsonData::Bool(true) => false,
    }
}
