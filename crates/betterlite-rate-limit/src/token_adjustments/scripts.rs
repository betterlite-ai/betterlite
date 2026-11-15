use betterlite_redis::cache_primitives::pipeline_increment::IncrementOperation;
use betterlite_redis::cache_primitives::protocol::RedisScalar;

pub const TOKEN_INCREMENT_SCRIPT: &str = "
local results = {}

for i = 1, #KEYS do
    local key = KEYS[i]
    local increment_value = tonumber(ARGV[i * 2 - 1])
    local ttl_seconds = tonumber(ARGV[i * 2])

    local new_value = redis.call('INCRBYFLOAT', key, increment_value)

    if ttl_seconds and ttl_seconds > 0 then
        local current_ttl = redis.call('TTL', key)
        if current_ttl == -1 then
            redis.call('EXPIRE', key, ttl_seconds)
        end
    end

    table.insert(results, new_value)
end

return results
";

// ast-grep-ignore: no-long-names
pub const WINDOW_GUARDED_TOKEN_INCREMENT_SCRIPT: &str = "
local results = {}
for i = 1, #KEYS, 2 do
    local window_key = KEYS[i]
    local counter_key = KEYS[i + 1]
    local arg_base = ((i - 1) / 2) * 3 + 1
    local expected_window_start = ARGV[arg_base]
    local increment = tonumber(ARGV[arg_base + 1])
    local ttl = tonumber(ARGV[arg_base + 2])
    local active_window_start = redis.call('GET', window_key)

    if active_window_start and active_window_start == expected_window_start then
        local new_counter = redis.call('INCRBY', counter_key, increment)
        local current_ttl = redis.call('TTL', counter_key)
        if current_ttl == -1 and ttl > 0 then
            redis.call('EXPIRE', counter_key, ttl)
        end
        table.insert(results, 1)
        table.insert(results, new_counter)
    else
        table.insert(results, 0)
        table.insert(results, tonumber(redis.call('GET', counter_key) or 0))
    end
end
return results
";

#[must_use]
pub fn token_increment_args(operations: &[IncrementOperation]) -> Vec<RedisScalar> {
    let mut args = Vec::with_capacity(operations.len().saturating_mul(2));
    for operation in operations {
        args.push(RedisScalar::Float(operation.increment_value));
        args.push(RedisScalar::Text(operation.ttl.unwrap_or(0).to_string()));
    }
    args
}

#[must_use]
pub fn window_guarded_args(
    expected_window_start: &str,
    operation: &IncrementOperation,
) -> Vec<RedisScalar> {
    // the guarded script calls INCRBY, which rejects a float text like "5.0"; Display writes a whole count as "5"
    vec![
        RedisScalar::Text(expected_window_start.to_owned()),
        RedisScalar::Text(operation.increment_value.to_string()),
        RedisScalar::Text(operation.ttl.unwrap_or(0).to_string()),
    ]
}
