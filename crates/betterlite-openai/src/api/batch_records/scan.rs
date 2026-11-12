use serde::de::Error as _;
use serde_json::{Deserializer, Value};

use super::{BatchRecordsError, BatchRow, DecodedRows, UnparsedTail, WireDecodeLimits};

const PREVIEW_CHARS: usize = 100;

// Python's str.strip() also trims the separators U+001C to U+001F, which Rust's White_Space leaves alone
const fn is_strip_space(character: char) -> bool {
    match character {
        '\u{1c}'..='\u{1f}' => true,
        other => other.is_whitespace(),
    }
}

fn unparsed(rows: Vec<BatchRow>, buffer: &str, error: serde_json::Error) -> DecodedRows {
    DecodedRows {
        rows,
        unparsed: Some(UnparsedTail {
            preview: buffer.chars().take(PREVIEW_CHARS).collect(),
            error,
        }),
    }
}

// LiteLLM retries json.loads at every newline (quadratic); text after a complete value can never be repaired by reading on, so one pass finds the same rows
pub(super) fn scan_rows(
    content: &str,
    limits: &WireDecodeLimits,
) -> Result<DecodedRows, BatchRecordsError> {
    let mut rows = Vec::new();
    let mut buffer = content;
    loop {
        let trimmed = buffer.trim_start_matches(is_strip_space);
        if trimmed.is_empty() {
            return Ok(DecodedRows {
                rows,
                unparsed: None,
            });
        }
        let mut stream = Deserializer::from_str(trimmed).into_iter::<Value>();
        let parsed = stream.next();
        let consumed = stream.byte_offset();
        let (value, after) = match (parsed, trimmed.split_at_checked(consumed)) {
            (Some(Ok(value)), Some((_, after))) => (value, after),
            (Some(Err(error)), _) => return Ok(unparsed(rows, buffer, error)),
            _ => {
                let error = serde_json::Error::custom("the batch content has no complete row");
                return Ok(unparsed(rows, buffer, error));
            }
        };
        let (line, next) = after.split_once('\n').unwrap_or((after, ""));
        if !line.chars().all(is_strip_space) {
            let error = serde_json::Error::custom("extra data after a complete batch row");
            return Ok(unparsed(rows, buffer, error));
        }
        if consumed > limits.max_event_bytes {
            return Err(BatchRecordsError::RowTooLarge {
                limit: limits.max_event_bytes,
            });
        }
        if rows.len() >= limits.max_items {
            return Err(BatchRecordsError::TooManyRows {
                limit: limits.max_items,
            });
        }
        rows.push(BatchRow::from(value));
        buffer = next;
    }
}
