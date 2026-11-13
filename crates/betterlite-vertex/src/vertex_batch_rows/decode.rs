use betterlite_models::decisions::Json;

use crate::vertex_batch_rows::correlation::{
    CorrelationInput, custom_id_from_row, decode_batch_correlation,
};
use crate::vertex_batch_rows::error::BatchRowsError;
use crate::vertex_batch_rows::types::{BatchKey, RowField, VertexBatchRow};

pub fn decode_vertex_batch_row(bytes: &[u8]) -> Result<VertexBatchRow, BatchRowsError> {
    match serde_json::from_slice(bytes) {
        Ok(row) => Ok(row),
        Err(source) => Err(BatchRowsError::Decode { source }),
    }
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
    match row.request.as_ref().and_then(RowField::value) {
        Some(Json::Object(request)) => {
            !row.status.as_deref().unwrap_or_default().is_empty() && request.contains_key("content")
        }
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => false,
    }
}

#[must_use]
pub fn is_generate_output_row(row: &VertexBatchRow) -> bool {
    if row.request.is_none() || row.response.is_none() || row.processed_time.is_none() {
        return false;
    }
    let generated = row.response_object().is_some_and(|response| {
        response.contains_key("candidates") || response.contains_key("promptFeedback")
    });
    generated || !row.status.as_deref().unwrap_or_default().is_empty()
}

pub fn split_vertex_batch_key(row: &VertexBatchRow) -> Result<BatchKey, BatchRowsError> {
    let Some(key) = row.key.as_deref() else {
        return Ok(BatchKey {
            custom_id: custom_id_from_row(row)?,
            index: 0,
            total: 1,
        });
    };
    let Some((custom_id, index, total)) = fan_out_suffix(key) else {
        return Ok(BatchKey {
            custom_id: decode_batch_correlation(CorrelationInput::Key(key.as_bytes()))?,
            index: 0,
            total: 1,
        });
    };
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
