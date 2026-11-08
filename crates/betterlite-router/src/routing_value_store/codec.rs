use std::io;

use betterlite_context::local_ttl_cache::CacheValue;

use super::{MAX_VALUE_BYTES, RouteValue, StoreError};

pub fn parse_route_value(bytes: &[u8]) -> Result<RouteValue, StoreError> {
    if bytes.len() > MAX_VALUE_BYTES {
        return Err(StoreError::ValueTooLarge {
            bytes: bytes.len(),
            limit: MAX_VALUE_BYTES,
        });
    }
    serde_json::from_slice(bytes).map_err(|source| StoreError::InvalidValue { source })
}

pub(crate) fn to_cache_value(value: &RouteValue) -> Result<Option<CacheValue>, StoreError> {
    // the size is counted before the encoding, so an oversized value never allocates its bytes
    let mut counter = ByteCounter::default();
    serde_json::to_writer(&mut counter, value).map_err(|source| StoreError::Encoding { source })?;
    if counter.bytes > MAX_VALUE_BYTES {
        return Ok(None);
    }
    let bytes = serde_json::to_vec(value).map_err(|source| StoreError::Encoding { source })?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|source| StoreError::Encoding { source })
}

pub(crate) fn from_cache_value(value: &CacheValue) -> Result<RouteValue, StoreError> {
    match value {
        CacheValue::Bytes(bytes) => parse_route_value(bytes),
        CacheValue::Credential(_) => Err(StoreError::NotARouteValue),
        CacheValue::Array(_)
        | CacheValue::Boolean(_)
        | CacheValue::Float(_)
        | CacheValue::Integer(_)
        | CacheValue::Null
        | CacheValue::Object(_)
        | CacheValue::Text(_) => {
            let bytes =
                serde_json::to_vec(value).map_err(|source| StoreError::Encoding { source })?;
            parse_route_value(&bytes)
        }
    }
}

#[derive(Default)]
struct ByteCounter {
    bytes: usize,
}

impl io::Write for ByteCounter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.bytes = self
            .bytes
            .checked_add(buf.len())
            .ok_or_else(|| io::Error::other("the route value size overflowed"))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
