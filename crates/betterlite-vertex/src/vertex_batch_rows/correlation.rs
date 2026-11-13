use std::str::from_utf8;

use percent_encoding::percent_decode_str;

use crate::vertex_batch_rows::error::BatchRowsError;
use crate::vertex_batch_rows::labels::{
    RAW_LABEL, RAW_LABEL_PREFIX, UNKNOWN_ID, custom_id_from_labels, decode_base32_text,
};
use crate::vertex_batch_rows::types::VertexBatchRow;

#[derive(Clone, Copy)]
pub enum CorrelationInput<'input> {
    Key(&'input [u8]),
    LabelChunks(&'input [&'input [u8]]),
}

pub fn decode_batch_correlation(input: CorrelationInput<'_>) -> Result<String, BatchRowsError> {
    match input {
        CorrelationInput::Key(bytes) => {
            let text = from_utf8(bytes).map_err(|source| BatchRowsError::Utf8 {
                field: "key",
                source,
            })?;
            let decoded =
                percent_decode_str(text)
                    .decode_utf8()
                    .map_err(|source| BatchRowsError::Utf8 {
                        field: "key",
                        source,
                    })?;
            Ok(decoded.into_owned())
        }
        CorrelationInput::LabelChunks(chunks) => decode_label_chunks(chunks),
    }
}

fn decode_label_chunks(chunks: &[&[u8]]) -> Result<String, BatchRowsError> {
    let Some(first) = chunks.first() else {
        return Ok(String::new());
    };
    let first_text = from_utf8(first).map_err(|source| BatchRowsError::Utf8 {
        field: RAW_LABEL,
        source,
    })?;
    let mut payload = String::new();
    for chunk in chunks {
        let text = from_utf8(chunk).map_err(|source| BatchRowsError::Utf8 {
            field: RAW_LABEL,
            source,
        })?;
        let Some(rest) = text.strip_prefix(RAW_LABEL_PREFIX) else {
            return Ok(first_text.to_owned());
        };
        payload.push_str(rest);
    }
    decode_base32_text(&payload).map_or_else(|| Ok(first_text.to_owned()), Ok)
}

pub fn custom_id_from_row(row: &VertexBatchRow) -> Result<String, BatchRowsError> {
    if let Some(key) = row.key.as_deref() {
        return decode_batch_correlation(CorrelationInput::Key(key.as_bytes()));
    }
    row.labels()
        .map_or_else(|| Ok(String::from(UNKNOWN_ID)), custom_id_from_labels)
}
