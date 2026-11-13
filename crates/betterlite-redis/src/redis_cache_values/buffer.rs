use std::sync::PoisonError;

use super::RedisCacheValues;

pub(crate) fn buffered_reaches_flush(values: &RedisCacheValues, key: String, text: String) -> bool {
    let mut buffer = values.buffer.lock().unwrap_or_else(PoisonError::into_inner);
    buffer.push((key, text));
    buffer.len() >= values.flush_size
}

pub(crate) fn remove_sent(values: &RedisCacheValues, sent: usize) {
    let mut buffer = values.buffer.lock().unwrap_or_else(PoisonError::into_inner);
    let count = sent.min(buffer.len());
    buffer.drain(..count);
}
