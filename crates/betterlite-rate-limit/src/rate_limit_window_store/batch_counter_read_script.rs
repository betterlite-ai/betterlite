use super::CounterValues;
use crate::limiter_records::{CacheCounterValue, CounterReplyError, ScriptReply, ScriptReplyKind};

pub const BATCH_COUNTER_READ_SCRIPT: &str = "return redis.call('MGET', unpack(KEYS))";

pub fn counter_read_values_from_reply(
    reply: &ScriptReply,
) -> Result<CounterValues, CounterReplyError> {
    let ScriptReply::Array(elements) = reply else {
        return Err(CounterReplyError::NotAnArray { kind: reply.kind() });
    };
    let mut values = Vec::with_capacity(elements.len());
    for element in elements {
        values.push(match element {
            ScriptReply::Nil => None,
            ScriptReply::Int(number) => Some(CacheCounterValue::Int(*number)),
            ScriptReply::Float(number) => Some(CacheCounterValue::Float(*number)),
            ScriptReply::Text(text) => Some(CacheCounterValue::Text(text.clone())),
            ScriptReply::Bytes(bytes) => Some(CacheCounterValue::Bytes(bytes.clone())),
            ScriptReply::Array(_) => {
                return Err(CounterReplyError::UnsupportedElement {
                    kind: ScriptReplyKind::Array,
                });
            }
        });
    }
    Ok(values)
}

mod tests;
