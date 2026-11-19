use betterlite_cache::backend::memory::MemoryCacheOps as _;
use betterlite_cache::dual_cache::{CacheOptions, Ttl};
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_models::decisions::Json;

use super::support;
use crate::auth_cache::WriteOptions;
use crate::auth_invalidation::ports::{
    AuthInvalidationCache, SpendCounterCache, UsageInvalidationCache,
};

const TEAM_KEY: &str = "team_id:team-1";
const TOKEN: &str = "hashed-key-1";
const COUNTER: &str = "spend:team_member:user-1:team-1";

#[test]
fn management_ttl_uses_the_ordinary_default_then_the_fallback() {
    let configured = support::auth_cache(Some(45.0_f64), 60.0_f64);
    assert_eq!(
        AuthInvalidationCache::management_ttl(&configured).to_bits(),
        45.0_f64.to_bits()
    );
    let fallback = support::auth_cache(None, 60.0_f64);
    assert_eq!(
        AuthInvalidationCache::management_ttl(&fallback).to_bits(),
        60.0_f64.to_bits()
    );
}

#[test]
fn write_management_keeps_the_payload_and_its_ttl() {
    let runtime = support::runtime();
    let cache = support::auth_cache(None, 60.0_f64);
    let payload = Json::Text("team".to_owned());
    runtime
        .block_on(AuthInvalidationCache::write_management(
            &cache,
            TEAM_KEY,
            &payload,
            Some(45.0_f64),
        ))
        .expect("the management write succeeds");
    let stored = cache
        .in_memory_cache_for(TEAM_KEY)
        .get_cache(TEAM_KEY)
        .expect("the memory read");
    assert!(stored.is_some(), "the management write reaches memory");
    let ttl = runtime
        .block_on(cache.tier(TEAM_KEY).async_get_ttl(TEAM_KEY))
        .expect("the ttl read");
    assert_eq!(ttl, Some(Ttl::Expiry(1045.5_f64)));
}

#[test]
fn delete_sync_clears_the_memory_tier() {
    let runtime = support::runtime();
    let cache = support::auth_cache(None, 60.0_f64);
    runtime
        .block_on(cache.async_set(
            Some(TOKEN),
            &Json::Text("value".to_owned()),
            WriteOptions::default(),
        ))
        .expect("the seed write succeeds");
    let memory = cache.in_memory_cache_for(TOKEN);
    assert!(memory.get_cache(TOKEN).expect("the memory read").is_some());
    AuthInvalidationCache::delete_sync(&cache, TOKEN).expect("the sync delete succeeds");
    assert!(memory.get_cache(TOKEN).expect("the memory read").is_none());
}

#[test]
fn delete_async_clears_the_memory_tier() {
    let runtime = support::runtime();
    let cache = support::auth_cache(None, 60.0_f64);
    runtime
        .block_on(cache.async_set(
            Some(TOKEN),
            &Json::Text("value".to_owned()),
            WriteOptions::default(),
        ))
        .expect("the seed write succeeds");
    let memory = cache.in_memory_cache_for(TOKEN);
    assert!(memory.get_cache(TOKEN).expect("the memory read").is_some());
    runtime
        .block_on(AuthInvalidationCache::delete_async(&cache, TOKEN))
        .expect("the async delete succeeds");
    assert!(memory.get_cache(TOKEN).expect("the memory read").is_none());
}

#[test]
fn auth_delete_memory_clears_the_memory_tier() {
    let cache = support::auth_cache(None, 60.0_f64);
    let memory = cache.in_memory_cache_for(TOKEN);
    memory
        .set_cache(TOKEN, CacheValue::Float(1.0_f64), Some(60.0_f64))
        .expect("the seed write succeeds");
    assert!(memory.get_cache(TOKEN).expect("the memory read").is_some());
    AuthInvalidationCache::delete_memory(&cache, TOKEN).expect("the memory delete succeeds");
    assert!(memory.get_cache(TOKEN).expect("the memory read").is_none());
}

#[test]
fn usage_delete_memory_clears_the_memory_tier() {
    let cache = support::dual_cache(None);
    cache
        .in_memory_cache
        .set_cache(TOKEN, CacheValue::Float(1.0_f64), Some(60.0_f64))
        .expect("the seed write succeeds");
    assert!(
        cache
            .in_memory_cache
            .get_cache(TOKEN)
            .expect("the memory read")
            .is_some()
    );
    UsageInvalidationCache::delete_memory(&cache, TOKEN).expect("the memory delete succeeds");
    assert!(
        cache
            .in_memory_cache
            .get_cache(TOKEN)
            .expect("the memory read")
            .is_none()
    );
}

#[test]
fn usage_delete_async_clears_the_memory_tier() {
    let runtime = support::runtime();
    let cache = support::dual_cache(None);
    cache.set_cache(
        TOKEN,
        &Json::Text("value".to_owned()),
        CacheOptions::default(),
    );
    assert!(cache.get_cache(TOKEN, true).is_some());
    runtime
        .block_on(UsageInvalidationCache::delete_async(&cache, TOKEN))
        .expect("the async delete succeeds");
    assert!(cache.get_cache(TOKEN, true).is_none());
}

#[test]
fn spend_set_memory_keeps_the_value_and_its_ttl() {
    let runtime = support::runtime();
    let cache = support::dual_cache(None);
    SpendCounterCache::set_memory(&cache, COUNTER, 7.0_f64, 60.0_f64)
        .expect("the spend memory write succeeds");
    let stored = cache
        .in_memory_cache
        .get_cache(COUNTER)
        .expect("the memory read");
    assert!(
        matches!(stored, Some(CacheValue::Float(value)) if value.to_bits() == 7.0_f64.to_bits()),
        "the spend counter lands in memory as a float"
    );
    let ttl = runtime
        .block_on(cache.async_get_ttl(COUNTER))
        .expect("the ttl read");
    assert_eq!(ttl, Some(Ttl::Expiry(1060.5_f64)));
}
