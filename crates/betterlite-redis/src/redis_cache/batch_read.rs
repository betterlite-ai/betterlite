use std::collections::HashMap;

use indexmap::IndexMap;
use redis::FromRedisValue as _;

use super::error::RedisCacheError;
use crate::cache_primitives::decode::{CachedValue, DecodeError, decode_cached_value};
use crate::cache_primitives::namespace::check_and_fix_namespace;
use crate::redis_live_client::LiveClientError;

pub struct BatchKey {
    pub original: String,
    pub namespaced: String,
}

#[must_use]
pub fn batch_keys(key_list: &[Option<&str>], namespace: Option<&str>) -> Vec<BatchKey> {
    key_list
        .iter()
        .filter_map(|key| *key)
        .map(|key| BatchKey {
            original: key.to_owned(),
            namespaced: check_and_fix_namespace(namespace, Some(key))
                .unwrap_or_else(|| key.to_owned()),
        })
        .collect()
}

// litellm builds `dict(zip(keys, replies))` first, so a duplicate key keeps the last reply and only that reply is decoded
pub fn decoded_batch(
    keys: &[BatchKey],
    replies: &[Option<Vec<u8>>],
) -> Result<HashMap<String, Option<CachedValue>>, DecodeError> {
    let mut latest: IndexMap<&str, Option<&[u8]>> = IndexMap::new();
    for (key, reply) in keys.iter().zip(replies) {
        latest.insert(key.original.as_str(), reply.as_deref());
    }
    let mut values = HashMap::with_capacity(latest.len());
    for (original, reply) in latest {
        values.insert(original.to_owned(), decode_cached_value(reply)?);
    }
    Ok(values)
}

pub(crate) fn read_replies(
    keys: &[BatchKey],
    replies: Vec<redis::Value>,
) -> Result<HashMap<String, Option<CachedValue>>, RedisCacheError> {
    let values = replies
        .into_iter()
        .map(Option::<Vec<u8>>::from_redis_value)
        .collect::<Result<Vec<_>, redis::ParsingError>>()
        .map_err(|source| LiveClientError::Redis(redis::RedisError::from(source)))?;
    Ok(decoded_batch(keys, &values)?)
}
