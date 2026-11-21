use std::collections::BTreeMap;

use super::support::{environment, object};
use crate::coordination_redis_settings::source::{
    CacheBackendKind, CoordinationRedisSource, coordination_redis_source,
};

#[test]
fn a_nonempty_block_wins_over_the_cache_and_the_environment() {
    let settings = object(r#"{"host": "h"}"#);
    assert_eq!(
        coordination_redis_source(
            Some(&settings),
            CacheBackendKind::Redis,
            &environment(&[("REDIS_HOST", "env-host")]),
        ),
        Some(CoordinationRedisSource::CoordinationRedis),
    );
}

#[test]
fn an_empty_block_falls_back_to_the_plain_redis_cache() {
    let settings = object("{}");
    assert_eq!(
        coordination_redis_source(
            Some(&settings),
            CacheBackendKind::Redis,
            &environment(&[("REDIS_HOST", "env-host")]),
        ),
        Some(CoordinationRedisSource::CacheBackend),
    );
}

#[test]
fn an_empty_block_without_a_redis_cache_falls_back_to_the_environment() {
    let settings = object("{}");
    assert_eq!(
        coordination_redis_source(
            Some(&settings),
            CacheBackendKind::Other,
            &environment(&[("REDIS_URL", "redis://cache:6379")]),
        ),
        Some(CoordinationRedisSource::Environment),
    );
}

#[test]
fn without_a_block_a_redis_cache_or_the_environment_there_is_no_source() {
    assert_eq!(
        coordination_redis_source(None, CacheBackendKind::Other, &BTreeMap::new()),
        None,
    );
    assert_eq!(
        coordination_redis_source(
            Some(&object("{}")),
            CacheBackendKind::Other,
            &BTreeMap::new()
        ),
        None,
    );
}

#[test]
fn a_block_whose_only_value_is_null_is_still_nonempty() {
    let settings = object(r#"{"host": null}"#);
    assert_eq!(
        coordination_redis_source(Some(&settings), CacheBackendKind::Other, &BTreeMap::new()),
        Some(CoordinationRedisSource::CoordinationRedis),
    );
}
