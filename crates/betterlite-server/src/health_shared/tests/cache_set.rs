#![cfg(test)]

use betterlite_models::decisions::Json;
use serde_json::Number;

use super::manager::SharedHealthCheckManager;
use super::settings::SharedHealthSettings;
use super::support::{
    CHECKER, Fault, SHARED_AT, SHARED_SECONDS, World, field, healthy_row, instant, number,
    probe_result, unhealthy_row, written_payload,
};

#[tokio::test]
async fn the_payload_holds_rows_counts_timestamp_and_checker() {
    let world = World::new(instant(SHARED_AT));
    world
        .manager()
        .cache_health_check_results(
            &[healthy_row("probe-ok")],
            &[unhealthy_row("probe-down", 503)],
        )
        .await;
    let call = world
        .cache
        .sets()
        .into_iter()
        .next()
        .expect("the cache set");
    assert_eq!(call.key, SharedHealthCheckManager::CACHE_KEY);
    assert_eq!(call.ttl, 300);
    assert!(!call.nx);
    let payload = written_payload(&world);
    assert_eq!(
        field(&payload, "healthy_endpoints"),
        Json::List(vec![Json::Object(healthy_row("probe-ok"))])
    );
    assert_eq!(
        field(&payload, "unhealthy_endpoints"),
        Json::List(vec![Json::Object(unhealthy_row("probe-down", 503))])
    );
    assert_eq!(
        field(&payload, "healthy_count"),
        Json::Number(Number::from(1_u32))
    );
    assert_eq!(
        field(&payload, "unhealthy_count"),
        Json::Number(Number::from(1_u32))
    );
    assert_eq!(field(&payload, "timestamp"), number(SHARED_SECONDS));
    assert_eq!(
        field(&payload, "checked_by"),
        Json::Text(CHECKER.to_owned())
    );
}

#[tokio::test]
async fn the_configured_ttl_rides_on_the_entry() {
    let world = World::new(instant(SHARED_AT));
    let settings = SharedHealthSettings {
        health_check_ttl: 45,
        lock_ttl: 60,
    };
    world
        .manager_with(settings)
        .cache_health_check_results(&[], &[])
        .await;
    assert_eq!(world.cache.sets()[0].ttl, 45);
}

#[tokio::test]
async fn the_written_value_reads_back_as_the_same_result() {
    let world = World::new(instant(SHARED_AT));
    let manager = world.manager();
    let live = probe_result();
    manager
        .cache_health_check_results(&live.healthy_endpoints, &live.unhealthy_endpoints)
        .await;
    let cached = manager
        .cached_health_check_results()
        .await
        .expect("the just-written entry is fresh");
    assert_eq!(cached.healthy_endpoints, live.healthy_endpoints);
    assert_eq!(cached.unhealthy_endpoints, live.unhealthy_endpoints);
    assert_eq!(cached.checked_by.as_deref(), Some(manager.pod_id.as_str()));
}

#[tokio::test]
async fn empty_rows_are_cached_with_zero_counts() {
    let world = World::new(instant(SHARED_AT));
    world.manager().cache_health_check_results(&[], &[]).await;
    let payload = written_payload(&world);
    assert_eq!(
        field(&payload, "healthy_count"),
        Json::Number(Number::from(0_u32))
    );
    assert_eq!(
        field(&payload, "unhealthy_count"),
        Json::Number(Number::from(0_u32))
    );
    assert_eq!(field(&payload, "healthy_endpoints"), Json::List(Vec::new()));
    assert_eq!(
        field(&payload, "unhealthy_endpoints"),
        Json::List(Vec::new())
    );
}

#[tokio::test]
async fn no_redis_writes_nothing() {
    let world = World::new(instant(SHARED_AT));
    world
        .local_manager()
        .cache_health_check_results(&[healthy_row("probe-ok")], &[])
        .await;
    assert!(world.ops.entries().is_empty());
}

#[tokio::test]
async fn a_failed_write_is_logged_not_raised() {
    let world = World::new(instant(SHARED_AT));
    world
        .cache
        .fail(Fault::Set, SharedHealthCheckManager::CACHE_KEY);
    world
        .manager()
        .cache_health_check_results(&[healthy_row("probe-ok")], &[])
        .await;
    assert_eq!(world.cache.sets().len(), 1);
    assert!(!world.cache.has(SharedHealthCheckManager::CACHE_KEY));
}
