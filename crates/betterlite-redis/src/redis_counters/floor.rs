use redis::cmd;

use crate::cache_primitives::decode::{INCREMENT_WITH_FLOOR_LUA, validate_lua_count};
use crate::cache_primitives::namespace::check_and_fix_namespace;
use crate::cache_primitives::protocol::{AsyncRedisCommands as _, RedisScalar};

use super::reply::{RedisReply, classify};
use super::{RedisCounters, RedisCountersError, Result};

pub(super) fn sync_floor(counters: &RedisCounters, key: &str, value: i64, ttl: i64) -> Result<i64> {
    let reply = counters.transport.sync_client.query(&floor_command(
        counters.namespace.as_deref(),
        key,
        value,
        ttl,
    ))?;
    lua_count(reply, "increment_with_floor")
}

pub(super) async fn eval_floor(
    counters: &RedisCounters,
    key: &str,
    value: i64,
    ttl: i64,
) -> Result<i64> {
    let namespaced = check_and_fix_namespace(counters.namespace.as_deref(), Some(key))
        .unwrap_or_else(|| key.to_owned());
    let client = counters.transport.async_client().await?;
    let arguments = [
        RedisScalar::Text(namespaced),
        RedisScalar::Text(value.to_string()),
        RedisScalar::Text(ttl.to_string()),
    ];
    match client.eval(INCREMENT_WITH_FLOOR_LUA, 1, &arguments).await {
        Ok(reply) => lua_count(reply, "async_increment_with_floor"),
        Err(error) => Err(RedisCountersError::Client(error)),
    }
}

fn floor_command(namespace: Option<&str>, key: &str, value: i64, ttl: i64) -> redis::Cmd {
    let namespaced =
        check_and_fix_namespace(namespace, Some(key)).unwrap_or_else(|| key.to_owned());
    let mut command = cmd("EVAL");
    command
        .arg(INCREMENT_WITH_FLOOR_LUA)
        .arg(1_i64)
        .arg(&namespaced)
        .arg(value.to_string())
        .arg(ttl.to_string());
    command
}

fn lua_count(reply: redis::Value, operation: &'static str) -> Result<i64> {
    match classify(reply) {
        RedisReply::Int(count) => Ok(count),
        RedisReply::Bulk(bytes) => Ok(validate_lua_count(&bytes)?),
        RedisReply::Nil
        | RedisReply::Array(_)
        | RedisReply::Double(_)
        | RedisReply::Unsupported => Err(RedisCountersError::UnexpectedReply { operation }),
    }
}
