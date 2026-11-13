use std::borrow::Cow;
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
        CorrelationInput::Key(bytes) => percent_decode(bytes)
            .decode_utf8()
            .map(Cow::into_owned)
            .map_err(|source| BatchRowsError::Utf8 {
                field: "key",
                source,
            }),
        CorrelationInput::LabelChunks(chunks) => {
            let mut texts = Vec::with_capacity(chunks.len());
            for chunk in chunks {
                texts.push(from_utf8(chunk).map_err(|source| BatchRowsError::Utf8 {
                    field: RAW_LABEL,
                    source,
                })?);
            }
            let Some(first) = texts.first() else {
                return Ok(String::new());
            };
            Ok(decode_gcp_label_value_chunks(&texts).unwrap_or_else(|| (*first).to_owned()))
        }
    }
}
