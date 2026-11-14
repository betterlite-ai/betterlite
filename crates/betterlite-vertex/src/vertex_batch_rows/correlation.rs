use std::str::from_utf8;

use percent_encoding::percent_decode;

use crate::vertex_batch_rows::error::BatchRowsError;
use crate::vertex_batch_rows::labels::{RAW_LABEL, decode_gcp_label_value_chunks};

#[derive(Clone, Copy)]
pub enum CorrelationInput<'input> {
    Key(&'input [u8]),
    LabelChunks(&'input [&'input [u8]]),
}

pub fn decode_batch_correlation(input: CorrelationInput<'_>) -> Result<String, BatchRowsError> {
    match input {
        CorrelationInput::Key(bytes) => String::from_utf8(percent_decode(bytes).collect())
            .map_err(|error| BatchRowsError::Utf8("key", error.utf8_error())),
        CorrelationInput::LabelChunks(chunks) => {
            let texts = chunks
                .iter()
                .map(|chunk| from_utf8(chunk))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|source| BatchRowsError::Utf8(RAW_LABEL, source))?;
            let Some(first) = texts.first() else {
                return Ok(String::new());
            };
            Ok(decode_gcp_label_value_chunks(&texts).unwrap_or_else(|| (*first).to_owned()))
        }
    }
}
