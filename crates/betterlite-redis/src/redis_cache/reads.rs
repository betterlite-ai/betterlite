use redis::cmd;

use super::RedisCache;
use super::error::{RedisCacheError, connection_failure};
use crate::cache_primitives::decode::{CachedValue, decode_cached_value};

pub(crate) enum Read {
    Value(Option<CachedValue>),
    Client(RedisCacheError),
    Failure(RedisCacheError),
}

pub(crate) async fn get_cached(cache: &RedisCache, key: &str) -> Read {
    let client = match cache.transport.async_client().await {
        Ok(client) => client,
        Err(source) if connection_failure(&source) => {
            return Read::Failure(RedisCacheError::from(source));
        }
        Err(source) => return Read::Client(RedisCacheError::from(source)),
    };
    let mut command = cmd("GET");
    command.arg(key);
    let reply: Option<Vec<u8>> = match client.query(&command).await {
        Ok(reply) => reply,
        Err(source) => return Read::Failure(RedisCacheError::from(source)),
    };
    match decode_cached_value(reply.as_deref()) {
        Ok(value) => Read::Value(value),
        Err(error) => Read::Failure(RedisCacheError::Decode(error)),
    }
}
