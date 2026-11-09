use std::str;

use serde_json::Deserializer;
use serde_json::value::RawValue;

use super::error::ForecastError;
use super::verdict::Verdict;

pub const MAX_REPLY_BYTES: usize = 0x0010_0000;
pub(super) const SCAN_BUDGET: usize = MAX_REPLY_BYTES.saturating_mul(16);

// same-length stand-ins so the decoder locates the object Python's raw_decode would have accepted
const MASKED_CONSTANTS: [(&[u8], &[u8]); 3] = [
    (b"NaN", b"0e0"),
    (b"Infinity", b"1e000000"),
    (b"-Infinity", b"-1e000000"),
];

pub fn parse_cap_reply(reply: &[u8]) -> Result<Verdict, ForecastError> {
    if reply.len() > MAX_REPLY_BYTES {
        return Err(ForecastError::ReplyTooLarge {
            bytes: reply.len(),
            limit: MAX_REPLY_BYTES,
        });
    }
    let content = str::from_utf8(reply).map_err(|source| ForecastError::ReplyNotText { source })?;
    let object = first_object(content, SCAN_BUDGET).ok_or(ForecastError::NoVerdictObject)?;
    let verdict: Verdict =
        serde_json::from_str(object).map_err(|source| ForecastError::VerdictSchema { source })?;
    verdict.validate()
}

pub(super) fn first_object(content: &str, budget: usize) -> Option<&str> {
    let masked = mask_constants(content);
    let mut remaining = budget;
    for (start, _) in content.match_indices('{') {
        match object_at(content, &masked, start) {
            Ok(object) => return Some(object),
            Err(consumed) => remaining = remaining.checked_sub(consumed)?,
        }
    }
    None
}

// a failed start is charged the byte the decoder stopped at, so a fast failure costs a few bytes, not the remainder
type Scan<'input> = Result<&'input str, usize>;

fn object_at<'input>(content: &'input str, masked: &str, start: usize) -> Scan<'input> {
    let tail = masked.get(start..).unwrap_or_default();
    let mut values = Deserializer::from_str(tail).into_iter::<&RawValue>();
    let decoded = match values.next() {
        Some(Ok(_)) => values.byte_offset(),
        Some(Err(error)) => {
            let base = tail
                .split('\n')
                .take(error.line().saturating_sub(1))
                .map(|part| part.len().saturating_add(1))
                .sum::<usize>();
            return Err(base.saturating_add(error.column()).min(tail.len()));
        }
        None => return Err(tail.len()),
    };
    content
        .get(start..start.saturating_add(decoded))
        .ok_or(decoded)
}

fn mask_constants(content: &str) -> String {
    let bytes = content.as_bytes();
    let mut masked = Vec::with_capacity(bytes.len());
    let mut index = 0_usize;
    while let Some(byte) = bytes.get(index).copied() {
        if let Some((replacement, length)) = masked_constant(bytes, index) {
            masked.extend_from_slice(replacement);
            // every replacement advances, so a degenerate zero-length one cannot stall the walk
            index = index.saturating_add(length.max(1));
            continue;
        }
        masked.push(byte);
        index = index.saturating_add(1);
    }
    String::from_utf8(masked).unwrap_or_default()
}

fn masked_constant(bytes: &[u8], index: usize) -> Option<(&'static [u8], usize)> {
    let before = index
        .checked_sub(1)
        .and_then(|previous| bytes.get(previous))
        .copied();
    if before.is_some_and(|byte| {
        !matches!(
            byte,
            b' ' | b'\t' | b'\n' | b'\r' | b'{' | b'[' | b',' | b':'
        )
    }) {
        return None;
    }
    MASKED_CONSTANTS.iter().find_map(|(token, replacement)| {
        let rest = bytes.get(index..)?;
        if !rest.starts_with(token) {
            return None;
        }
        let after = bytes.get(index.saturating_add(token.len())).copied();
        let terminated = after
            .is_none_or(|byte| matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | b'}' | b']' | b','));
        terminated.then_some((*replacement, token.len()))
    })
}
