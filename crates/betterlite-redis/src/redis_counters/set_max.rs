use std::str;

use crate::cache_primitives::namespace::check_and_fix_namespace;
use crate::cache_primitives::protocol::{AsyncRedisCommands as _, RedisScalar};

use super::reply::{RedisReply, classify};
use super::{RedisCounters, RedisCountersError};

pub const SET_MAX_LUA: &str = "local cur = redis.call('GET', KEYS[1]) if cur == false or tonumber(cur) < tonumber(ARGV[1]) then redis.call('SET', KEYS[1], ARGV[1]) if tonumber(ARGV[2]) > 0 then redis.call('EXPIRE', KEYS[1], ARGV[2]) end return ARGV[1] end return cur";

pub(crate) async fn set_max_body(
    counters: &RedisCounters,
    key: &str,
    value: f64,
    ttl: Option<i64>,
) -> Result<Option<f64>, RedisCountersError> {
    let unexpected = || RedisCountersError::UnexpectedReply {
        operation: "async_set_max",
    };
    if !value.is_finite() {
        return Err(RedisCountersError::NonFiniteValue);
    }
    let used_ttl = ttl.unwrap_or(counters.default_ttl);
    let namespaced = check_and_fix_namespace(counters.namespace.as_deref(), Some(key))
        .unwrap_or_else(|| key.to_owned());
    let client = counters.transport.async_client().await?;
    let scalars = [
        RedisScalar::Text(namespaced),
        RedisScalar::Text(value.to_string()),
        RedisScalar::Text(used_ttl.to_string()),
    ];
    let reply = client.eval(SET_MAX_LUA, 1, &scalars).await?;
    match classify(reply) {
        RedisReply::Nil => Ok(None),
        RedisReply::Bulk(bytes) => {
            let Ok(text) = str::from_utf8(&bytes) else {
                return Err(unexpected());
            };
            text.parse().map(Some).map_err(|_source| unexpected())
        }
        RedisReply::Int(_)
        | RedisReply::Array(_)
        | RedisReply::Double(_)
        | RedisReply::Unsupported => Err(unexpected()),
    }
}
