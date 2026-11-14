#![cfg(test)]

use betterlite_redis::cache_primitives::protocol::RedisScalar;

use super::support::operation;
use crate::token_adjustments::scripts::{
    TOKEN_INCREMENT_SCRIPT, WINDOW_GUARDED_TOKEN_INCREMENT_SCRIPT, token_increment_args,
    window_guarded_args,
};

// LiteLLM's TOKEN_INCREMENT_SCRIPT at parallel_request_limiter_v3.py:400-425, comments removed
const PORTED_TOKEN_INCREMENT_SCRIPT: &str = "
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

// LiteLLM's WINDOW_GUARDED_TOKEN_INCREMENT_SCRIPT at parallel_request_limiter_v3.py:310-335
const PORTED_WINDOW_GUARDED_SCRIPT: &str = "
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

#[test]
fn the_token_increment_script_is_the_ported_text_with_comments_removed() {
    assert_eq!(TOKEN_INCREMENT_SCRIPT, PORTED_TOKEN_INCREMENT_SCRIPT);
}

#[test]
fn the_token_increment_script_increments_by_a_float() {
    assert!(TOKEN_INCREMENT_SCRIPT.contains("redis.call('INCRBYFLOAT', key, increment_value)"));
    assert!(!TOKEN_INCREMENT_SCRIPT.contains("'INCRBY'"));
}

#[test]
fn the_token_increment_script_reads_a_ttl_beside_each_key() {
    assert!(TOKEN_INCREMENT_SCRIPT.contains("local increment_value = tonumber(ARGV[i * 2 - 1])"));
    assert!(TOKEN_INCREMENT_SCRIPT.contains("local ttl_seconds = tonumber(ARGV[i * 2])"));
}

#[test]
fn the_token_increment_script_sets_a_ttl_only_when_the_key_has_none() {
    let ttl_read = TOKEN_INCREMENT_SCRIPT
        .find("local current_ttl = redis.call('TTL', key)")
        .expect("the TTL read");
    let expire = TOKEN_INCREMENT_SCRIPT
        .find("redis.call('EXPIRE', key, ttl_seconds)")
        .expect("the TTL write");
    assert!(TOKEN_INCREMENT_SCRIPT.contains("if ttl_seconds and ttl_seconds > 0 then"));
    assert!(TOKEN_INCREMENT_SCRIPT.contains("if current_ttl == -1 then"));
    assert!(ttl_read < expire);
}

#[test]
fn the_window_guarded_script_is_the_ported_text() {
    assert_eq!(
        WINDOW_GUARDED_TOKEN_INCREMENT_SCRIPT,
        PORTED_WINDOW_GUARDED_SCRIPT
    );
}

#[test]
fn the_window_guarded_script_takes_window_and_counter_key_pairs() {
    assert!(WINDOW_GUARDED_TOKEN_INCREMENT_SCRIPT.contains("for i = 1, #KEYS, 2 do"));
    assert!(WINDOW_GUARDED_TOKEN_INCREMENT_SCRIPT.contains("local window_key = KEYS[i]"));
    assert!(WINDOW_GUARDED_TOKEN_INCREMENT_SCRIPT.contains("local counter_key = KEYS[i + 1]"));
}

#[test]
fn the_window_guarded_script_takes_a_start_increment_ttl_triple() {
    let script = WINDOW_GUARDED_TOKEN_INCREMENT_SCRIPT;
    assert!(script.contains("local arg_base = ((i - 1) / 2) * 3 + 1"));
    assert!(script.contains("local expected_window_start = ARGV[arg_base]"));
    assert!(script.contains("local increment = tonumber(ARGV[arg_base + 1])"));
    assert!(script.contains("local ttl = tonumber(ARGV[arg_base + 2])"));
}

#[test]
fn the_window_guarded_script_checks_the_window_before_incrementing() {
    let script = WINDOW_GUARDED_TOKEN_INCREMENT_SCRIPT;
    let check = script
        .find("if active_window_start and active_window_start == expected_window_start then")
        .expect("the window check");
    let increment = script
        .find("redis.call('INCRBY', counter_key, increment)")
        .expect("the counter increment");
    assert!(check < increment);
}

#[test]
fn the_window_guarded_script_returns_the_flag_and_the_counter() {
    let script = WINDOW_GUARDED_TOKEN_INCREMENT_SCRIPT;
    assert!(script.contains("table.insert(results, 1)"));
    assert!(script.contains("table.insert(results, new_counter)"));
    assert!(script.contains("table.insert(results, 0)"));
    assert!(
        script.contains("table.insert(results, tonumber(redis.call('GET', counter_key) or 0))")
    );
    assert!(!script.contains("INCRBYFLOAT"));
}

#[test]
fn the_window_guarded_script_preserves_an_existing_ttl() {
    assert!(
        WINDOW_GUARDED_TOKEN_INCREMENT_SCRIPT.contains("if current_ttl == -1 and ttl > 0 then")
    );
}

#[test]
fn the_token_arguments_pair_each_increment_with_its_ttl() {
    let operations = [
        operation("{a:1}:tokens", 3.5, Some(60)),
        operation("{b:1}:tokens", -2.0, None),
    ];
    assert_eq!(
        token_increment_args(&operations),
        vec![
            RedisScalar::Float(3.5),
            RedisScalar::Text("60".to_owned()),
            RedisScalar::Float(-2.0),
            RedisScalar::Text("0".to_owned()),
        ]
    );
}

#[test]
fn the_guarded_arguments_are_the_start_the_increment_and_the_ttl() {
    assert_eq!(
        window_guarded_args("100", &operation("{a:1}:tokens", -5.0, None)),
        vec![
            RedisScalar::Text("100".to_owned()),
            RedisScalar::Float(-5.0),
            RedisScalar::Text("0".to_owned()),
        ]
    );
    assert_eq!(
        window_guarded_args("101", &operation("{a:1}:tokens", 5.0, Some(30))),
        vec![
            RedisScalar::Text("101".to_owned()),
            RedisScalar::Float(5.0),
            RedisScalar::Text("30".to_owned()),
        ]
    );
}
