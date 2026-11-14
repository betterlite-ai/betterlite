use super::codec::decode_line;
use super::error::FileUploadValidationError;
use super::failure::{FILE_PARAM, FILE_TOO_LARGE, FailureFields, INVALID_REQUEST};
use super::json::{self, Scan};
use super::line_shape::LineShape;
use super::source::FileBody;
use super::{exceeds, megabytes};
use crate::api::api_boundaries::{
    ApiBoundariesInput, ApiLimits, EventBoundary, ItemsBoundary, JsonBoundary,
};
use std::io::Read;
const JSONL_EXTENSION: &str = ".jsonl";
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatchValidationFailure {
    TooLarge {
        size_bytes: u128,
        limit_mb: i64,
    },
    WrongExtension {
        filename: String,
    },
    Empty,
    InvalidJsonLine {
        line_number: usize,
    },
    LineNotObject {
        line_number: usize,
    },
    MissingLineKey {
        line_number: usize,
        key: &'static str,
        line_shape: LineShape,
    },
}
impl BatchValidationFailure {
    pub fn fields(&self) -> FailureFields {
        let (status, param) = match self {
            Self::TooLarge { .. } => (FILE_TOO_LARGE, FILE_PARAM.to_owned()),
            Self::MissingLineKey { key, .. } => (INVALID_REQUEST, (*key).to_owned()),
            Self::WrongExtension { .. }
            | Self::Empty
            | Self::InvalidJsonLine { .. }
            | Self::LineNotObject { .. } => (INVALID_REQUEST, FILE_PARAM.to_owned()),
        };
        FailureFields {
            message: self.message(),
            status,
            param,
        }
    }
    fn message(&self) -> String {
        match self {
            Self::TooLarge {
                size_bytes,
                limit_mb,
            } => format!(
                "Batch input file is {} MB, which exceeds the configured max_batch_file_size_mb of {limit_mb} MB. The file was not forwarded to the provider.",
                megabytes(*size_bytes)
            ),
            Self::WrongExtension { filename } => format!(
                "Invalid file format for Batch API: '{filename}'. Batch input files must be .jsonl files. The file was not forwarded to the provider."
            ),
            Self::Empty => {
                "Batch input file has no request lines. The file was not forwarded to the provider."
                    .to_owned()
            }
            Self::InvalidJsonLine { line_number } => format!(
                "Batch input file line {line_number} is not valid JSON. The file was not forwarded to the provider."
            ),
            Self::LineNotObject { line_number } => format!(
                "Batch input file line {line_number} must be a JSON object. The file was not forwarded to the provider."
            ),
            Self::MissingLineKey {
                line_number,
                key,
                line_shape,
            } => format!(
                "Missing required parameter: '{key}' (batch input file line {line_number}). {}. The file was not forwarded to the provider.",
                line_shape.hint()
            ),
        }
    }
}
pub fn check_batch_file_upload(
    filename: Option<&str>,
    body: &mut FileBody<'_>,
    max_batch_file_size_mb: Option<i64>,
    line_shape: LineShape,
    limits: ApiLimits,
) -> Result<Option<BatchValidationFailure>, FileUploadValidationError> {
    let Some(filename) = filename else {
        return Ok(Some(BatchValidationFailure::WrongExtension {
            filename: String::new(),
        }));
    };
    if !filename.to_lowercase().ends_with(JSONL_EXTENSION) {
        return Ok(Some(BatchValidationFailure::WrongExtension {
            filename: filename.to_owned(),
        }));
    }
    if let Some(limit_mb) = max_batch_file_size_mb {
        let size_bytes = body.size()?;
        body.reset()?;
        if exceeds(size_bytes, limit_mb) {
            return Ok(Some(BatchValidationFailure::TooLarge {
                size_bytes,
                limit_mb,
            }));
        }
    }
    body.reset()?;
    let failure = scan(body, line_shape, limits)?;
    body.reset()?;
    Ok(failure)
}
pub fn decode_file_upload_validation(
    bytes: &[u8],
    line_shape: LineShape,
    limits: ApiLimits,
) -> Result<Option<BatchValidationFailure>, FileUploadValidationError> {
    limits.check(ApiBoundariesInput::Json(JsonBoundary {
        known_bytes: bytes.len(),
    }))?;
    let mut content_lines = 0usize;
    for (index, line) in (ByteLines { rest: bytes }).enumerate() {
        let line_number = index.saturating_add(1);
        limits.check(ApiBoundariesInput::Event(EventBoundary {
            known_bytes: line.len(),
        }))?;
        if line
            .iter()
            .all(|byte| matches!(*byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c))
        {
            continue;
        }
        content_lines = content_lines.saturating_add(1);
        limits.check(ApiBoundariesInput::Items(ItemsBoundary {
            known_count: content_lines,
        }))?;
        if let Some(failure) = scan_line(line_number, line, line_shape) {
            return Ok(Some(failure));
        }
    }
    match content_lines {
        0 => Ok(Some(BatchValidationFailure::Empty)),
        _ => Ok(None),
    }
}
fn scan(
    body: &mut FileBody<'_>,
    line_shape: LineShape,
    limits: ApiLimits,
) -> Result<Option<BatchValidationFailure>, FileUploadValidationError> {
    match body {
        FileBody::Bytes(bytes) => decode_file_upload_validation(*bytes, line_shape, limits),
        FileBody::Seekable(reader) => {
            // a seekable body is read once, bounded by the JSON cap, before the same scan runs
            let ceiling =
                u64::try_from(limits.max_json_bytes.saturating_add(1)).unwrap_or(u64::MAX);
            let mut bytes = Vec::new();
            reader.take(ceiling).read_to_end(&mut bytes)?;
            decode_file_upload_validation(&bytes, line_shape, limits)
        }
    }
}
fn scan_line(
    line_number: usize,
    line: &[u8],
    line_shape: LineShape,
) -> Option<BatchValidationFailure> {
    let Some(text) = decode_line(line) else {
        return Some(BatchValidationFailure::InvalidJsonLine { line_number });
    };
    match json::scan(&text, line_shape) {
        Scan::Valid => None,
        Scan::NotObject => Some(BatchValidationFailure::LineNotObject { line_number }),
        Scan::Invalid => Some(BatchValidationFailure::InvalidJsonLine { line_number }),
        Scan::Missing(key) => Some(BatchValidationFailure::MissingLineKey {
            line_number,
            key,
            line_shape,
        }),
    }
}
struct ByteLines<'a> {
    rest: &'a [u8],
}
impl<'a> Iterator for ByteLines<'a> {
    type Item = &'a [u8];
    fn next(&mut self) -> Option<&'a [u8]> {
        if self.rest.is_empty() {
            return None;
        }
        let mut end = self.rest.len();
        let mut skip = end;
        for (index, byte) in self.rest.iter().enumerate() {
            if *byte == b'\n' {
                end = index;
                skip = index.saturating_add(1);
                break;
            }
            if *byte == b'\r' {
                end = index;
                skip = match self.rest.get(index.saturating_add(1)) {
                    Some(&b'\n') => index.saturating_add(2),
                    _ => index.saturating_add(1),
                };
                break;
            }
        }
        let line = self.rest.get(..end)?;
        self.rest = self.rest.get(skip..)?;
        Some(line)
    }
}
