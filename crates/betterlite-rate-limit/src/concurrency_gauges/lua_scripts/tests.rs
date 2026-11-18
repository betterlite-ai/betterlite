#![cfg(test)]

use betterlite_redis::cache_primitives::protocol::RedisScalar;
use rstest::rstest;

use crate::concurrency_gauges::PARALLEL_SLOT_TTL;
use crate::concurrency_gauges::tests::support::{gauge, key_list};
use crate::limiter_records::PARALLEL_SLOT_TTL_SECONDS;

use super::{
    PARALLEL_ACQUIRE_SCRIPT, PARALLEL_COUNT_SCRIPT, PARALLEL_RELEASE_SCRIPT, acquire_script_args,
    count_script_args, release_script_args,
};

fn balanced_blocks(script: &str) -> bool {
    let mut depth: usize = 0;
    for line in script.lines() {
        let trimmed = line.trim();
        let opens = (trimmed.starts_with("for ") && trimmed.ends_with(" do"))
            || (trimmed.starts_with("if ") && trimmed.ends_with(" then"));
        if opens {
            depth = depth.saturating_add(1);
        }
        if trimmed == "end" {
            let Some(shallower) = depth.checked_sub(1) else {
                return false;
            };
            depth = shallower;
        }
    }
    depth == 0
}

#[rstest]
#[case::acquire(PARALLEL_ACQUIRE_SCRIPT)]
#[case::release(PARALLEL_RELEASE_SCRIPT)]
#[case::count(PARALLEL_COUNT_SCRIPT)]
fn gauge_scripts_keep_lua_block_structure(#[case] script: &str) {
    assert!(balanced_blocks(script));
}

#[test]
fn acquire_script_checks_every_gauge_before_registering_any_slot() {
    let validation = PARALLEL_ACQUIRE_SCRIPT
        .find("return { 1, i, in_flight, limit }")
        .expect("the over-limit return");
    let registration = PARALLEL_ACQUIRE_SCRIPT
        .find("redis.call('ZADD', KEYS[i], now, slot_id)")
        .expect("the slot write");
    assert!(validation < registration);
    assert!(PARALLEL_ACQUIRE_SCRIPT.contains("local time_reply = redis.call('TIME')"));
    assert!(PARALLEL_ACQUIRE_SCRIPT.contains("local now = tonumber(time_reply[1])"));
    assert!(
        PARALLEL_ACQUIRE_SCRIPT
            .contains("redis.call('ZREMRANGEBYSCORE', KEYS[i], '-inf', now - slot_ttl)")
    );
    assert!(PARALLEL_ACQUIRE_SCRIPT.contains("if in_flight + 1 > limit then"));
    assert!(PARALLEL_ACQUIRE_SCRIPT.contains("redis.call('EXPIRE', KEYS[i], slot_ttl)"));
    assert!(PARALLEL_ACQUIRE_SCRIPT.contains("local results = { 0 }"));
}

#[test]
fn release_script_removes_the_slot_without_reading_the_clock() {
    assert!(PARALLEL_RELEASE_SCRIPT.contains("redis.call('ZREM', KEYS[i], ARGV[i])"));
    assert!(
        PARALLEL_RELEASE_SCRIPT.contains("table.insert(results, redis.call('ZCARD', KEYS[i]))")
    );
    assert!(!PARALLEL_RELEASE_SCRIPT.contains("TIME"));
    assert!(!PARALLEL_RELEASE_SCRIPT.contains("ZADD"));
    assert!(!PARALLEL_RELEASE_SCRIPT.contains("ZREMRANGEBYSCORE"));
}

#[test]
fn count_script_prunes_by_server_time_before_counting() {
    assert!(PARALLEL_COUNT_SCRIPT.contains("local time_reply = redis.call('TIME')"));
    assert!(
        PARALLEL_COUNT_SCRIPT
            .contains("redis.call('ZREMRANGEBYSCORE', KEYS[i], '-inf', now - tonumber(ARGV[i]))")
    );
    assert!(PARALLEL_COUNT_SCRIPT.contains("table.insert(results, redis.call('ZCARD', KEYS[i]))"));
    assert!(!PARALLEL_COUNT_SCRIPT.contains("ZADD"));
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
