use betterlite_models::decisions::Json;

use crate::vertex_batch_rows::correlation::{CorrelationInput, decode_batch_correlation};
use crate::vertex_batch_rows::error::BatchRowsError;
use crate::vertex_batch_rows::labels::{UNKNOWN_ID, custom_id_from_labels};
use crate::vertex_batch_rows::types::{BatchKey, RowField, VertexBatchRow};

pub fn decode_vertex_batch_row(bytes: &[u8]) -> Result<VertexBatchRow, BatchRowsError> {
    // serde would read a JSON array as a row, but LiteLLM only accepts an object
    if bytes.trim_ascii_start().first() == Some(&b'[') {
        return Err(BatchRowsError::NotAnObject);
    }
    serde_json::from_slice(bytes).map_err(|source| BatchRowsError::Decode { source })
}

#[must_use]
pub fn is_embeddings_output_row(row: &VertexBatchRow) -> bool {
    if row.request.is_none() {
        return false;
    }
    let embedding = row
        .response_object()
        .and_then(|response| response.get("embedding"));
    if matches!(embedding, Some(Json::Object(_))) {
        return true;
    }
    let Some(RowField::Value(Json::Object(request))) = row.request.as_ref() else {
        return false;
    };
    !row.status.as_deref().unwrap_or_default().is_empty() && request.contains_key("content")
}

#[must_use]
pub fn is_generate_output_row(row: &VertexBatchRow) -> bool {
    let marked = row.request.is_some() && row.response.is_some() && row.processed_time.is_some();
    let generated = row.response_object().is_some_and(|response| {
        response.contains_key("candidates") || response.contains_key("promptFeedback")
    });
    marked && (generated || !row.status.as_deref().unwrap_or_default().is_empty())
}

pub fn split_vertex_batch_key(row: &VertexBatchRow) -> Result<BatchKey, BatchRowsError> {
    let Some(key) = row.key.as_deref() else {
        let custom_id = row
            .labels()
            .map_or_else(|| Ok(String::from(UNKNOWN_ID)), custom_id_from_labels)?;
        return Ok(BatchKey {
            custom_id,
            index: 0,
            total: 1,
        });
    };
    let (custom_id, index, total) = fan_out_suffix(key).unwrap_or((key, 0, 1));
    Ok(BatchKey {
        custom_id: decode_batch_correlation(CorrelationInput::Key(custom_id.as_bytes()))?,
        index,
        total,
    })
}

fn fan_out_suffix(key: &str) -> Option<(&str, u32, u32)> {
    let (custom_id, tail) = key.split_once('#')?;
    let (index, total) = tail.split_once('/')?;
    Some((custom_id, position(index)?, position(total)?))
}

fn position(text: &str) -> Option<u32> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}
