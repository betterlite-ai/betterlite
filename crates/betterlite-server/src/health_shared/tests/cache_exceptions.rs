#![cfg(test)]

use betterlite_models::decisions::Json;

use super::manager::SharedHealthCheckManager;
use super::support::{
    RecordingSleeper, SHARED_AT, World, cached_text, empty_call, healthy_row, instant, keys,
    probe_result, unhealthy_row, written_payload,
};

#[tokio::test]
async fn the_in_process_return_keeps_the_exception_map() {
    let world = World::new(instant(SHARED_AT));
    let manager = world.manager();
    let mut sleeper = RecordingSleeper::default();
    let result = manager
        .perform_shared_health_check(empty_call(), world.probe.as_ref(), &mut sleeper)
        .await
        .expect("the check runs");
    assert_eq!(result, probe_result());
    assert_eq!(result.exceptions_by_model_id.len(), 1);
    assert!(result.exceptions_by_model_id.contains_key("probe-down"));
}

#[tokio::test]
async fn the_cached_payload_excludes_the_exception_map_but_keeps_the_rows() {
    let world = World::new(instant(SHARED_AT));
    let manager = world.manager();
    let mut sleeper = RecordingSleeper::default();
    manager
        .perform_shared_health_check(empty_call(), world.probe.as_ref(), &mut sleeper)
        .await
        .expect("the check runs");
    let payload = written_payload(&world);
    assert_eq!(
        keys(&payload),
        vec![
            "healthy_endpoints",
            "unhealthy_endpoints",
            "healthy_count",
            "unhealthy_count",
            "timestamp",
            "checked_by",
        ]
    );
    assert_eq!(
        payload.get("healthy_endpoints"),
        Some(&Json::List(vec![Json::Object(healthy_row("probe-ok"))]))
    );
    assert_eq!(
        payload.get("unhealthy_endpoints"),
        Some(&Json::List(vec![Json::Object(unhealthy_row(
            "probe-down",
            503
        ))]))
    );
    let Some(Json::List(entries)) = payload.get("unhealthy_endpoints") else {
        panic!("the payload carries the unhealthy rows");
    };
    let Some(Json::Object(entry)) = entries.first() else {
        panic!("the unhealthy row is an object");
    };
    assert!(entry.get("exception").is_none());
}

#[tokio::test]
async fn a_cache_hit_returns_the_rows_without_the_exception_map() {
    let world = World::new(instant(SHARED_AT));
    let manager = world.manager();
    let mut sleeper = RecordingSleeper::default();
    manager
        .perform_shared_health_check(empty_call(), world.probe.as_ref(), &mut sleeper)
        .await
        .expect("the first check runs");
    let cached = manager
        .perform_shared_health_check(empty_call(), world.probe.as_ref(), &mut sleeper)
        .await
        .expect("the second call reads the cache");
    assert_eq!(cached.healthy_endpoints, probe_result().healthy_endpoints);
    assert_eq!(
        cached.unhealthy_endpoints,
        probe_result().unhealthy_endpoints
    );
    assert!(cached.exceptions_by_model_id.is_empty());
    assert_eq!(world.probe.calls().len(), 1);
}

#[tokio::test]
async fn a_prepopulated_cache_entry_is_read_without_exceptions() {
    let world = World::new(instant(SHARED_AT));
    world.cache.put_json(
        SharedHealthCheckManager::CACHE_KEY,
        &cached_text(1_704_067_200.0, "pod_7"),
    );
    let manager = world.manager();
    let mut sleeper = RecordingSleeper::default();
    let result = manager
        .perform_shared_health_check(empty_call(), world.probe.as_ref(), &mut sleeper)
        .await
        .expect("the cached rows are returned");
    assert_eq!(result.healthy_endpoints, vec![healthy_row("cached-ok")]);
    assert_eq!(
        result.unhealthy_endpoints,
        vec![unhealthy_row("cached-down", 500)]
    );
    assert!(result.exceptions_by_model_id.is_empty());
    assert!(world.probe.calls().is_empty());
}
