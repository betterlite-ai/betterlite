use serde::de::DeserializeOwned;

use crate::budget_permission_rows::JsonData;
use crate::resource_rows::{ResourceRowsError, json_value_is_set, parse_data};

#[derive(Debug, thiserror::Error)]
pub enum ManagedBatchRecordError {
    #[error(transparent)]
    Stored(#[from] ResourceRowsError),
}

// B1035 owns the batch contract; the caller names it, since betterlite-openai sits above this crate
pub fn decode_managed_batch_record<Batch>(
    bytes: &[u8],
) -> Result<Option<Batch>, ManagedBatchRecordError>
where
    Batch: DeserializeOwned,
{
    let stored = parse_data(bytes)?;
    if !json_value_is_set(&stored) {
        return Ok(None);
    }
    let value = match stored {
        JsonData::String(inner) => parse_data(inner.as_bytes())?,
        other => other,
    };
    serde_json::to_value(value)
        .and_then(serde_json::from_value::<Batch>)
        .map(Some)
        .map_err(|source| ManagedBatchRecordError::Stored(ResourceRowsError::Json(source)))
}
