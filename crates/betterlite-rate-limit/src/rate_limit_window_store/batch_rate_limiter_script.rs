use betterlite_redis::cache_primitives::protocol::RedisScalar;

use super::{WindowStore, WindowStoreError};
use crate::limiter_records::{CacheCounterValue, counter_values_from_reply};

pub const BATCH_RATE_LIMITER_SCRIPT: &str = "
local results = {}
local now = tonumber(ARGV[1])
local window_size = tonumber(ARGV[2])

for i = 1, #KEYS, 2 do
    local window_key = KEYS[i]
    local counter_key = KEYS[i + 1]
    local increment_value = 1

    local window_start = redis.call('GET', window_key)
    if not window_start or (now - tonumber(window_start)) >= window_size then
        local prefix = string.sub(window_key, 1, -(#':window') - 1)
        redis.call('DEL', prefix .. ':requests', prefix .. ':tokens')
        redis.call('SET', window_key, tostring(now))
        redis.call('SET', counter_key, increment_value)
        redis.call('EXPIRE', window_key, window_size)
        redis.call('EXPIRE', counter_key, window_size)
        table.insert(results, tostring(now))
        table.insert(results, increment_value)
    else
        local counter = redis.call('INCR', counter_key)
        local current_ttl = redis.call('TTL', counter_key)
        if current_ttl == -1 then
            redis.call('EXPIRE', counter_key, window_size)
        end
        table.insert(results, window_start)
        table.insert(results, counter)
    end
end

return results
";

#[must_use]
pub fn window_script_args(now_int: i64, window_size: i64) -> Vec<RedisScalar> {
    vec![
        RedisScalar::Text(now_int.to_string()),
        RedisScalar::Text(window_size.to_string()),
    ]
}

pub async fn run_batch_rate_limiter_script(
    store: &WindowStore,
    keys: &[String],
    args: &[RedisScalar],
) -> Result<Vec<CacheCounterValue>, WindowStoreError> {
    let Some(script) = store.rate_limiter_script.as_deref() else {
        return Err(WindowStoreError::MissingScript {
            name: "batch_rate_limiter_script",
        });
    };
    let reply = script.run(keys, args).await?;
    Ok(counter_values_from_reply(&reply)?)
}

mod tests;
