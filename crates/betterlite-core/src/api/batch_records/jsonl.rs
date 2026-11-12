use betterlite_openai::api::batch_records::{
    BatchRecordsError as WireError, BatchRow, DecodedRows, WireDecodeLimits, decode_batch_records,
};
use tracing::error;

use super::{BatchFileContent, BatchRecordsError, InMemoryFile, ReplacedContent, ReplacedFile};
use crate::api::api_boundaries::ApiLimits;

fn decode(limits: &ApiLimits, bytes: &[u8]) -> Result<DecodedRows, WireError> {
    decode_batch_records(
        bytes,
        &WireDecodeLimits {
            max_json_bytes: limits.max_json_bytes,
            max_event_bytes: limits.max_event_bytes,
            max_items: limits.max_items,
        },
    )
}

pub(super) fn parse_jsonl(
    limits: &ApiLimits,
    content: &[u8],
) -> Result<Vec<BatchRow>, BatchRecordsError> {
    let decoded = decode(limits, content)?;
    match decoded.unparsed {
        Some(tail) => {
            error!(
                "error parsing final buffer: {}..., error: {}",
                tail.preview, tail.error
            );
            Err(BatchRecordsError::Unparsed(tail))
        }
        None => Ok(decoded.rows),
    }
}

pub(super) fn replace_model(
    limits: &ApiLimits,
    file: BatchFileContent,
    new_model_name: &str,
) -> Result<ReplacedFile, BatchRecordsError> {
    let BatchFileContent::Bytes(bytes) = file else {
        return Ok(ReplacedFile::default());
    };
    let decoded = match decode(limits, &bytes) {
        Ok(decoded) => decoded,
        Err(WireError::NotUtf8(_)) => return Ok(ReplacedFile::default()),
        Err(other) => return Err(BatchRecordsError::Decode(other)),
    };
    if let Some(tail) = decoded.unparsed {
        error!("error parsing trailing batch content: {}...", tail.preview);
        return Ok(ReplacedFile {
            content: ReplacedContent::Original,
            logged_preview: Some(tail.preview),
        });
    }
    if decoded.rows.is_empty() {
        return Ok(ReplacedFile::default());
    }
    let lines = decoded
        .rows
        .into_iter()
        .map(|row| serde_json::to_string(&row.with_model(new_model_name)))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ReplacedFile {
        content: ReplacedContent::Rewritten(InMemoryFile {
            content: lines.join("\n").into_bytes(),
            name: "modified_file.jsonl",
            content_type: "application/jsonl",
        }),
        logged_preview: None,
    })
}
