use crate::cache_primitives::decode::decoded_counts;
use crate::cache_primitives::namespace::check_and_fix_namespace;

use super::reply::{RedisReply, classify};
use super::{Counts, RedisCounters, RedisCountersError, Result};

pub(super) fn batch_get_counts(counters: &RedisCounters, keys: &[String]) -> Result<Counts> {
    decode_mget(
        counters
            .transport
            .mget(&namespaced_keys(counters.namespace.as_deref(), keys))?,
        "batch_get_counts",
    )
}

pub(super) async fn read_counts(counters: &RedisCounters, keys: &[String]) -> Result<Counts> {
    let namespaced = namespaced_keys(counters.namespace.as_deref(), keys);
    match counters.transport.async_mget(&namespaced).await {
        Ok(replies) => decode_mget(replies, "async_batch_get_counts"),
        Err(error) => Err(RedisCountersError::Reuse(error)),
    }
}

fn namespaced_keys(namespace: Option<&str>, keys: &[String]) -> Vec<String> {
    keys.iter()
        .map(|key| {
            check_and_fix_namespace(namespace, Some(key.as_str())).unwrap_or_else(|| key.clone())
        })
        .collect()
}

fn decode_mget(replies: Vec<redis::Value>, operation: &'static str) -> Result<Counts> {
    let mut values = Vec::with_capacity(replies.len());
    for reply in replies {
        match classify(reply) {
            RedisReply::Nil => values.push(None),
            RedisReply::Bulk(bytes) => values.push(Some(bytes)),
            RedisReply::Int(_)
            | RedisReply::Array(_)
            | RedisReply::Double(_)
            | RedisReply::Unsupported => {
                return Err(RedisCountersError::UnexpectedReply { operation });
            }
        }
    }
    Ok(decoded_counts(&values)?)
}
