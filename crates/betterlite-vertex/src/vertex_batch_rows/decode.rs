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
    Ok(serde_json::from_slice(bytes)?)
}

#[must_use]
pub fn is_embeddings_output_row(row: &VertexBatchRow) -> bool {
    let embedded = matches!(
        row.response
            .object()
            .and_then(|fields| fields.get("embedding")),
        Some(Json::Object(_))
    );
    let failed = row
        .request
        .object()
        .is_some_and(|fields| fields.contains_key("content"));
    row.request != RowField::Absent
        && (embedded
            || (failed
                && row
                    .status
                    .as_deref()
                    .is_some_and(|status| !status.is_empty())))
}

#[must_use]
pub fn is_generate_output_row(row: &VertexBatchRow) -> bool {
    if [&row.request, &row.response, &row.processed_time].contains(&&RowField::Absent) {
        return false;
    }
    let generated = row.response.object().is_some_and(|fields| {
        fields.contains_key("candidates") || fields.contains_key("promptFeedback")
    });
    generated
        || row
            .status
            .as_deref()
            .is_some_and(|status| !status.is_empty())
}

pub fn split_vertex_batch_key(row: &VertexBatchRow) -> Result<BatchKey, BatchRowsError> {
    let Some(key) = row.key.as_deref() else {
        let declared = row.request.object().and_then(|fields| fields.get("labels"));
        let custom_id = if let Some(Json::Object(labels)) = declared {
            custom_id_from_labels(labels)?
        } else {
            String::from(UNKNOWN_ID)
        };
        return Ok(BatchKey {
            custom_id,
            index: 0,
            total: 1,
        });
    };
    // LiteLLM's pattern is `[^#]*#\d+/\d+`, so a second `#` or `/` leaves the key whole
    let suffix = key.split_once('#').and_then(|(custom_id, tail)| {
        let (index, total) = tail.split_once('/')?;
        let digits = |text: &str| text.bytes().all(|byte| byte.is_ascii_digit());
        if !digits(index) || !digits(total) {
            return None;
        }
        Some((custom_id, index.parse().ok()?, total.parse().ok()?))
    });
    let (custom_id, index, total) = suffix.unwrap_or((key, 0, 1));
    Ok(BatchKey {
        custom_id: decode_batch_correlation(CorrelationInput::Key(custom_id.as_bytes()))?,
        index,
        total,
    })
}
