use std::collections::HashMap;

use redis::FromRedisValue as _;

use super::error::RedisCacheValuesError;
use crate::cache_primitives::decode::{CachedValue, decode_cached_value};
use crate::redis_client_reuse::transport::RedisTransport;

pub(crate) fn read_value(
    transport: &RedisTransport,
    key: &str,
) -> Result<Option<Vec<u8>>, RedisCacheValuesError> {
    let mut command = redis::cmd("GET");
    command.arg(key);
    Ok(transport.sync_client.query(&command)?)
}

pub(crate) fn batch_values(
    originals: &[&str],
    replies: Vec<redis::Value>,
) -> Result<HashMap<String, Option<CachedValue>>, RedisCacheValuesError> {
    let mut values = HashMap::new();
    for (original, reply) in originals.iter().zip(replies) {
        let bytes = Option::<Vec<u8>>::from_redis_value(reply)?;
        let decoded = decode_cached_value(bytes.as_deref())?;
        values.insert((*original).to_owned(), decoded);
    }
    Ok(values)
}
