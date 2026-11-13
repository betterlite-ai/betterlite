use std::borrow::Cow;
use std::str::from_utf8;

use percent_encoding::percent_decode_str;

use crate::vertex_batch_rows::error::BatchRowsError;
use crate::vertex_batch_rows::labels::{RAW_LABEL, RAW_LABEL_PREFIX, decode_base32_text};

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
            percent_decode_str(text)
                .decode_utf8()
                .map(Cow::into_owned)
                .map_err(|source| BatchRowsError::Utf8 {
                    field: "key",
                    source,
                })
        }
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
            let mut payload = String::new();
            for text in &texts {
                let Some(rest) = text.strip_prefix(RAW_LABEL_PREFIX) else {
                    return Ok((*first).to_owned());
                };
                payload.push_str(rest);
            }
            Ok(decode_base32_text(&payload).unwrap_or_else(|| (*first).to_owned()))
        }
    }
}
