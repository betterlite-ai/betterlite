use std::str;

use serde_json::Deserializer;
use serde_json::value::RawValue;

use super::error::ForecastError;
use super::verdict::Verdict;

pub const MAX_REPLY_BYTES: usize = 0x0010_0000;

pub fn parse_cap_reply(reply: &[u8]) -> Result<Verdict, ForecastError> {
    if reply.len() > MAX_REPLY_BYTES {
        return Err(ForecastError::ReplyTooLarge {
            bytes: reply.len(),
            limit: MAX_REPLY_BYTES,
        });
    }
    let content = str::from_utf8(reply).map_err(|source| ForecastError::ReplyNotText { source })?;
    let object = first_object(content).ok_or(ForecastError::NoVerdictObject)?;
    let verdict: Verdict =
        serde_json::from_str(object).map_err(|source| ForecastError::VerdictSchema { source })?;
    verdict.validate()
}

fn first_object(content: &str) -> Option<&str> {
    content
        .match_indices('{')
        .find_map(|(start, _)| object_at(content, start))
}

fn object_at(content: &str, start: usize) -> Option<&str> {
    let tail = content.get(start..)?;
    let mut values = Deserializer::from_str(tail).into_iter::<&RawValue>();
    values.next()?.ok()?;
    let end = values.byte_offset();
    tail.get(..end)
}
