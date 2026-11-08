use serde_json::value::RawValue;

use super::RedisCacheValues;
use super::error::RedisCacheValuesError;

pub(crate) fn buffered_reaches_flush(values: &RedisCacheValues, key: String, text: String) -> bool {
    let mut buffer = match values.buffer.lock() {
        Ok(buffer) => buffer,
        Err(poisoned) => poisoned.into_inner(),
    };
    buffer.push((key, text));
    buffer.len() >= values.flush_size
}

pub(crate) fn buffered_entries(values: &RedisCacheValues) -> Vec<(String, String)> {
    match values.buffer.lock() {
        Ok(buffer) => buffer.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

pub(crate) fn serialize_entries(
    entries: &[(String, String)],
) -> Result<Vec<(String, Box<RawValue>)>, RedisCacheValuesError> {
    entries
        .iter()
        .map(|(key, text)| {
            RawValue::from_string(text.clone())
                .map(|value| (key.clone(), value))
                .map_err(RedisCacheValuesError::Serialize)
        })
        .collect()
}

pub(crate) fn remove_sent(values: &RedisCacheValues, sent: usize) {
    let mut buffer = match values.buffer.lock() {
        Ok(buffer) => buffer,
        Err(poisoned) => poisoned.into_inner(),
    };
    let count = sent.min(buffer.len());
    buffer.drain(..count);
}
