use std::str;

use redis::Value;

use super::error::RedisListOperationsError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LpopResult {
    Missing,
    Item(String),
    ItemBytes(Vec<u8>),
    Items(Vec<String>),
    ItemsBytes(Vec<Vec<u8>>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LpopPipelineEntry {
    Missing,
    Items(Vec<String>),
    ItemsBytes(Vec<Vec<u8>>),
}

pub(crate) fn decode_reply(value: &Value) -> Result<LpopResult, RedisListOperationsError> {
    if let Value::Array(items) = value {
        return Ok(decode_popped(&popped_items(items, "async_lpop")?));
    }
    let bytes = reply_bytes(value, "async_lpop")?;
    Ok(bytes.map_or(LpopResult::Missing, single))
}

pub(crate) fn decode_popped(items: &[Vec<u8>]) -> LpopResult {
    let Some(decoded) = texts(items) else {
        return LpopResult::ItemsBytes(items.to_vec());
    };
    LpopResult::Items(decoded)
}

pub(crate) fn decode_group(items: &[Vec<u8>]) -> LpopPipelineEntry {
    if items.is_empty() {
        return LpopPipelineEntry::Missing;
    }
    texts(items).map_or_else(
        || LpopPipelineEntry::ItemsBytes(items.to_vec()),
        LpopPipelineEntry::Items,
    )
}

pub(crate) fn popped_items(
    values: &[Value],
    operation: &'static str,
) -> Result<Vec<Vec<u8>>, RedisListOperationsError> {
    let mut popped = Vec::with_capacity(values.len());
    for value in values {
        let Some(bytes) = reply_bytes(value, operation)? else {
            return Err(RedisListOperationsError::UnexpectedReply { operation });
        };
        popped.push(bytes.to_vec());
    }
    Ok(popped)
}

pub(crate) fn reply_bytes<'reply>(
    value: &'reply Value,
    operation: &'static str,
) -> Result<Option<&'reply [u8]>, RedisListOperationsError> {
    if matches!(value, Value::Nil) {
        return Ok(None);
    }
    if let Value::BulkString(bytes) = value {
        return Ok(Some(bytes));
    }
    if let Value::SimpleString(text) = value {
        return Ok(Some(text.as_bytes()));
    }
    Err(RedisListOperationsError::UnexpectedReply { operation })
}

fn single(bytes: &[u8]) -> LpopResult {
    match str::from_utf8(bytes) {
        Ok(text) => LpopResult::Item(text.to_owned()),
        Err(_source) => LpopResult::ItemBytes(bytes.to_vec()),
    }
}

fn texts(items: &[Vec<u8>]) -> Option<Vec<String>> {
    let mut decoded = Vec::with_capacity(items.len());
    for item in items {
        decoded.push(str::from_utf8(item).ok()?.to_owned());
    }
    Some(decoded)
}
