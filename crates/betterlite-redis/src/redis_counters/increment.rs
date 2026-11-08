use std::time::Instant;

use redis::cmd;

use crate::redis_client_reuse::service_event::ServiceEvent;

use super::{RedisCounters, RedisCountersError};

// BaseCache.get_ttl: int() truncates toward zero, NaN falls back, and an infinity raises OverflowError
pub(crate) fn resolve_ttl(ttl: Option<f64>, default_ttl: i64) -> Result<i64, RedisCountersError> {
    let Some(seconds) = ttl else {
        return Ok(default_ttl);
    };
    if seconds.is_nan() {
        return Ok(default_ttl);
    }
    if seconds.is_infinite() {
        return Err(RedisCountersError::InvalidTtl);
    }
    let truncated = seconds.trunc();
    format!("{truncated:.0}")
        .parse()
        .map_err(|_source| RedisCountersError::InvalidTtl)
}

pub(crate) fn increment_steps(
    counters: &RedisCounters,
    key: &str,
    value: i64,
    set_ttl: i64,
) -> Result<i64, RedisCountersError> {
    let mut increment = cmd("INCRBY");
    increment.arg(key).arg(value);
    let increment_started = Instant::now();
    let count: i64 = counters.transport.sync_client.query(&increment)?;
    counters.hooks.success(&ServiceEvent {
        call_type: "increment_cache",
        duration: increment_started.elapsed(),
        key: Some(key.to_owned()),
    });

    let mut ask_ttl = cmd("TTL");
    ask_ttl.arg(key);
    let ttl_started = Instant::now();
    let current_ttl: i64 = counters.transport.sync_client.query(&ask_ttl)?;
    counters.hooks.success(&ServiceEvent {
        call_type: "increment_cache_ttl",
        duration: ttl_started.elapsed(),
        key: Some(key.to_owned()),
    });

    if current_ttl == -1 {
        let mut expire = cmd("EXPIRE");
        expire.arg(key).arg(set_ttl);
        let expire_started = Instant::now();
        counters.transport.sync_client.query::<()>(&expire)?;
        counters.hooks.success(&ServiceEvent {
            call_type: "increment_cache_expire",
            duration: expire_started.elapsed(),
            key: Some(key.to_owned()),
        });
    }
    Ok(count)
}
