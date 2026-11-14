use serde_json::Deserializer;
use serde_json::value::RawValue;
use thiserror::Error;

pub const MAX_REPLY_BYTES: usize = 0x0010_0000;
const SCAN_BUDGET: usize = MAX_REPLY_BYTES.saturating_mul(16);

// same-length stand-ins; all three parse as finite negative numbers, which validate_hours rejects
const MASKED_CONSTANTS: [(&str, &str); 3] = [
    ("NaN", "-1 "),
    ("Infinity", "-1000000"),
    ("-Infinity", "-10000000"),
];

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ExtractionError {
    #[error("the reply is {bytes} bytes, over the {limit} byte limit")]
    TooLarge { bytes: usize, limit: usize },
    #[error("the reply used up the scan budget before an object was found")]
    ScanBudget,
}

// B394's extractor is pub(super) outside this task's scope, so the estimator mirrors its scan
pub fn first_json_object(content: &str) -> Result<&str, ExtractionError> {
    if content.len() > MAX_REPLY_BYTES {
        return Err(ExtractionError::TooLarge {
            bytes: content.len(),
            limit: MAX_REPLY_BYTES,
        });
    }
    let masked = mask_constants(content);
    let mut remaining = SCAN_BUDGET;
    for (start, _) in content.match_indices('{') {
        let tail = masked.get(start..).unwrap_or_default();
        let mut values = Deserializer::from_str(tail).into_iter::<&RawValue>();
        let cost = match values.next() {
            Some(Ok(_)) => {
                let end = start.saturating_add(values.byte_offset());
                if let Some(object) = content.get(start..end) {
                    return Ok(object);
                }
                end
            }
            // a failed start is charged the byte the decoder stopped at, so a fast failure stays cheap
            Some(Err(error)) => failure_offset(tail, error.line(), error.column()),
            None => tail.len(),
        };
        remaining = remaining
            .checked_sub(cost)
            .ok_or(ExtractionError::ScanBudget)?;
    }
    Ok(content.trim())
}

fn failure_offset(tail: &str, line: usize, column: usize) -> usize {
    let lines_before = tail
        .split('\n')
        .take(line.saturating_sub(1))
        .map(|part| part.len().saturating_add(1))
        .sum::<usize>();
    lines_before.saturating_add(column).min(tail.len())
}

fn mask_constants(content: &str) -> String {
    let bytes = content.as_bytes().to_vec();
    let mut masked = content.to_owned();
    for (token, _) in MASKED_CONSTANTS {
        for (index, _) in content.match_indices(token) {
            mask_at(&bytes, &mut masked, index);
        }
    }
    masked
}

// the typed parse masks only value positions, so a constant written inside reasoning text stays as it is
pub fn mask_value_constants(content: &str) -> String {
    let bytes = content.as_bytes().to_vec();
    let mut masked = content.to_owned();
    let mut index = 0;
    let mut in_string = false;
    while let Some(&byte) = bytes.get(index) {
        if in_string {
            if byte == b'\\' {
                index = index.saturating_add(2);
                continue;
            }
            if byte == b'"' {
                in_string = false;
            }
            index = index.saturating_add(1);
            continue;
        }
        if byte == b'"' {
            in_string = true;
            index = index.saturating_add(1);
            continue;
        }
        let end = mask_at(&bytes, &mut masked, index).unwrap_or(index);
        index = end.max(index.saturating_add(1));
    }
    masked
}

fn mask_at(bytes: &[u8], masked: &mut String, index: usize) -> Option<usize> {
    let (token, stand_in) = MASKED_CONSTANTS.into_iter().find(|(token, _)| {
        bytes
            .get(index..)
            .is_some_and(|tail| tail.starts_with(token.as_bytes()))
    })?;
    let end = index.saturating_add(token.len());
    let opens = index
        .checked_sub(1)
        .and_then(|previous| bytes.get(previous))
        .is_none_or(|byte| b" \t\n\r{[,:".contains(byte));
    let closes = bytes
        .get(end)
        .is_none_or(|byte| b" \t\n\r}],".contains(byte));
    if !opens || !closes {
        return None;
    }
    masked.replace_range(index..end, stand_in);
    Some(end)
}
