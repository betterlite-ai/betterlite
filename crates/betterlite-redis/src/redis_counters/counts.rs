use crate::cache_primitives::decode::decoded_counts;
use crate::cache_primitives::namespace::check_and_fix_namespace;

use super::reply::{RedisReply, classify};
use super::{RedisCounters, RedisCountersError};

pub(crate) fn batch_get_counts(
    counters: &RedisCounters,
    key_list: &[String],
) -> Result<Vec<Option<i64>>, RedisCountersError> {
    let namespaced = namespaced_keys(counters.namespace.as_deref(), key_list);
    let replies = counters.transport.mget(&namespaced)?;
    decode_mget(replies, "batch_get_counts")
}

pub(crate) async fn async_batch_get_counts(
    counters: &RedisCounters,
    key_list: &[String],
) -> Result<Vec<Option<i64>>, RedisCountersError> {
    let namespaced = namespaced_keys(counters.namespace.as_deref(), key_list);
    let replies = counters.transport.async_mget(&namespaced).await?;
    decode_mget(replies, "async_batch_get_counts")
}

fn namespaced_keys(namespace: Option<&str>, keys: &[String]) -> Vec<String> {
    keys.iter()
        .map(|key| {
            check_and_fix_namespace(namespace, Some(key.as_str())).unwrap_or_else(|| key.clone())
        })
        .collect()
}

fn decode_mget(
    replies: Vec<redis::Value>,
    operation: &'static str,
) -> Result<Vec<Option<i64>>, RedisCountersError> {
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
