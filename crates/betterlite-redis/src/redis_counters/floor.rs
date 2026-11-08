use redis::cmd;

use crate::cache_primitives::decode::{INCREMENT_WITH_FLOOR_LUA, validate_lua_count};
use crate::cache_primitives::namespace::check_and_fix_namespace;
use crate::cache_primitives::protocol::{AsyncRedisCommands as _, RedisScalar};

use super::reply::{RedisReply, classify};
use super::{RedisCounters, RedisCountersError};

pub(crate) fn increment_with_floor(
    counters: &RedisCounters,
    key: &str,
    value: i64,
    ttl: i64,
) -> Result<i64, RedisCountersError> {
    let namespaced = check_and_fix_namespace(counters.namespace.as_deref(), Some(key))
        .unwrap_or_else(|| key.to_owned());
    let mut command = cmd("EVAL");
    command
        .arg(INCREMENT_WITH_FLOOR_LUA)
        .arg(1_i64)
        .arg(&namespaced)
        .arg(value.to_string())
        .arg(ttl.to_string());
    let reply: redis::Value = counters.transport.sync_client.query(&command)?;
    lua_count(classify(reply), "increment_with_floor")
}

pub(crate) async fn async_increment_with_floor(
    counters: &RedisCounters,
    key: &str,
    value: i64,
    ttl: i64,
) -> Result<i64, RedisCountersError> {
    let namespaced = check_and_fix_namespace(counters.namespace.as_deref(), Some(key))
        .unwrap_or_else(|| key.to_owned());
    let client = counters.transport.async_client().await?;
    let scalars = [
        RedisScalar::Text(namespaced),
        RedisScalar::Text(value.to_string()),
        RedisScalar::Text(ttl.to_string()),
    ];
    let reply = client.eval(INCREMENT_WITH_FLOOR_LUA, 1, &scalars).await?;
    lua_count(classify(reply), "async_increment_with_floor")
}

fn lua_count(reply: RedisReply, operation: &'static str) -> Result<i64, RedisCountersError> {
    match reply {
        RedisReply::Int(count) => Ok(count),
        RedisReply::Bulk(bytes) => Ok(validate_lua_count(&bytes)?),
        RedisReply::Nil
        | RedisReply::Array(_)
        | RedisReply::Double(_)
        | RedisReply::Unsupported => Err(RedisCountersError::UnexpectedReply { operation }),
    }
}
