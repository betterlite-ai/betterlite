#![cfg(test)]

use betterlite_redis::cache_primitives::protocol::RedisScalar;

use crate::concurrency_gauges::PARALLEL_SLOT_TTL;
use crate::concurrency_gauges::tests::support::{gauge, key_list};
use crate::limiter_records::PARALLEL_SLOT_TTL_SECONDS;

use super::{
    PARALLEL_ACQUIRE_SCRIPT, PARALLEL_COUNT_SCRIPT, PARALLEL_RELEASE_SCRIPT, acquire_script_args,
    count_script_args, release_script_args,
};

// LiteLLM's PARALLEL_ACQUIRE_SCRIPT at parallel_request_limiter_v3.py:337-369, verbatim
const PORTED_ACQUIRE_SCRIPT: &str = "
-- Atomic check-and-acquire for the max_parallel_requests concurrency gauge.
-- Each gauge key is a sorted set of per-request slot ids scored by acquire
-- time (Redis server clock). In-flight requests are counted by ZCARD after
-- pruning slots older than the slot TTL, so unlike the windowed RPM/TPM
-- counters the gauge is never reset while requests are in flight, a
-- rejected request never occupies a slot, and a slot leaked by a crashed
-- worker self-heals after the slot TTL even under continuous traffic.
--
-- KEYS: one gauge zset key per descriptor.
-- ARGV: per-key triples (limit, slot_ttl_seconds, slot_id).
-- Success: { 0, in_flight_1, ... }. Over-limit: { 1, key_index, in_flight, limit }.
local time_reply = redis.call('TIME')
local now = tonumber(time_reply[1])
for i = 1, #KEYS do
    local limit = tonumber(ARGV[(i - 1) * 3 + 1])
    local slot_ttl = tonumber(ARGV[(i - 1) * 3 + 2])
    redis.call('ZREMRANGEBYSCORE', KEYS[i], '-inf', now - slot_ttl)
    local in_flight = redis.call('ZCARD', KEYS[i])
    if in_flight + 1 > limit then
        return { 1, i, in_flight, limit }
    end
end
local results = { 0 }
for i = 1, #KEYS do
    local slot_ttl = tonumber(ARGV[(i - 1) * 3 + 2])
    local slot_id = ARGV[(i - 1) * 3 + 3]
    redis.call('ZADD', KEYS[i], now, slot_id)
    redis.call('EXPIRE', KEYS[i], slot_ttl)
    table.insert(results, redis.call('ZCARD', KEYS[i]))
end
return results
";

// LiteLLM's PARALLEL_RELEASE_SCRIPT at parallel_request_limiter_v3.py:371-384, verbatim
const PORTED_RELEASE_SCRIPT: &str = "
-- Release one slot per gauge key by removing this request's slot id.
-- ZREM of an absent member (or key) is a no-op, so a release without a
-- matching acquire (proxy-side rejection, double-fired callback, slot
-- already expired) can never free a slot owned by another request.
-- KEYS: gauge zset keys. ARGV: per-key slot_id.
-- Returns the remaining in-flight count per key.
local results = {}
for i = 1, #KEYS do
    redis.call('ZREM', KEYS[i], ARGV[i])
    table.insert(results, redis.call('ZCARD', KEYS[i]))
end
return results
";

// LiteLLM's PARALLEL_COUNT_SCRIPT at parallel_request_limiter_v3.py:386-398, verbatim
const PORTED_COUNT_SCRIPT: &str = "
-- Read the current in-flight count per gauge key (prunes expired slots
-- first so leaked slots do not inflate the reading).
-- KEYS: gauge zset keys. ARGV: per-key slot_ttl_seconds.
local time_reply = redis.call('TIME')
local now = tonumber(time_reply[1])
local results = {}
for i = 1, #KEYS do
    redis.call('ZREMRANGEBYSCORE', KEYS[i], '-inf', now - tonumber(ARGV[i]))
    table.insert(results, redis.call('ZCARD', KEYS[i]))
end
return results
";

#[test]
fn acquire_script_checks_every_gauge_before_registering_any_slot() {
    assert_eq!(PARALLEL_ACQUIRE_SCRIPT, PORTED_ACQUIRE_SCRIPT);
    let validation = PARALLEL_ACQUIRE_SCRIPT
        .find("return { 1, i, in_flight, limit }")
        .expect("the over-limit return");
    let registration = PARALLEL_ACQUIRE_SCRIPT
        .find("redis.call('ZADD', KEYS[i], now, slot_id)")
        .expect("the slot write");
    assert!(validation < registration);
}

#[test]
fn release_script_removes_the_slot_without_reading_the_clock() {
    assert_eq!(PARALLEL_RELEASE_SCRIPT, PORTED_RELEASE_SCRIPT);
    assert!(!PARALLEL_RELEASE_SCRIPT.contains("TIME"));
}

#[test]
fn count_script_prunes_by_server_time_before_counting() {
    assert_eq!(PARALLEL_COUNT_SCRIPT, PORTED_COUNT_SCRIPT);
    let pruned = PARALLEL_COUNT_SCRIPT
        .find("redis.call('ZREMRANGEBYSCORE'")
        .expect("the slot prune");
    let counted = PARALLEL_COUNT_SCRIPT
        .find("redis.call('ZCARD'")
        .expect("the slot count");
    assert!(pruned < counted);
}

#[test]
fn acquire_script_args_writes_limit_ttl_slot_triples() {
    let args = acquire_script_args(&[gauge("g1", 4), gauge("g2", 9)], "slot-a");
    assert_eq!(
        args,
        vec![
            RedisScalar::Text("4".to_owned()),
            RedisScalar::Text("3600".to_owned()),
            RedisScalar::Text("slot-a".to_owned()),
            RedisScalar::Text("9".to_owned()),
            RedisScalar::Text("3600".to_owned()),
            RedisScalar::Text("slot-a".to_owned()),
        ]
    );
}

#[test]
fn count_script_args_are_one_ttl_per_gauge() {
    let args = count_script_args(&[gauge("g1", 4), gauge("g2", 9)]);
    assert_eq!(
        args,
        vec![
            RedisScalar::Text("3600".to_owned()),
            RedisScalar::Text("3600".to_owned()),
        ]
    );
}

#[test]
fn release_script_args_repeat_the_slot_id_per_key() {
    let args = release_script_args(&key_list(&["g1", "g2"]), "slot-a");
    assert_eq!(
        args,
        vec![
            RedisScalar::Text("slot-a".to_owned()),
            RedisScalar::Text("slot-a".to_owned()),
        ]
    );
}

#[test]
fn the_slot_ttl_matches_the_shared_records_constant() {
    assert_eq!(PARALLEL_SLOT_TTL.as_secs(), 3600);
    assert_eq!(PARALLEL_SLOT_TTL_SECONDS, 3600);
}
