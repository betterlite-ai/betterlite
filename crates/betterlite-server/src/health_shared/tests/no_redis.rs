#![cfg(test)]

use super::support::{
    RecordingSleeper, SHARED_AT, World, empty_call, instant, probe_error, probe_result,
};

#[tokio::test]
async fn the_local_check_runs_at_once_without_a_lock() {
    let world = World::new(instant(SHARED_AT));
    let manager = world.local_manager();
    let mut sleeper = RecordingSleeper::default();
    let result = manager
        .perform_shared_health_check(empty_call(), world.probe.as_ref(), &mut sleeper)
        .await
        .expect("the local check runs");
    assert_eq!(result, probe_result());
    assert!(sleeper.delays().is_empty());
    assert_eq!(world.ops.entries(), vec!["probe".to_owned()]);
}

#[tokio::test]
async fn the_local_check_returns_the_probe_failure() {
    let world = World::new(instant(SHARED_AT));
    world.probe.queue(Err(probe_error()));
    let manager = world.local_manager();
    let mut sleeper = RecordingSleeper::default();
    let error = manager
        .perform_shared_health_check(empty_call(), world.probe.as_ref(), &mut sleeper)
        .await
        .expect_err("the probe failure is returned");
    assert!(
        std::error::Error::source(&error).is_some(),
        "the cycle failure is kept as the source"
    );
    assert!(sleeper.delays().is_empty());
    assert_eq!(world.ops.entries(), vec!["probe".to_owned()]);
}

#[tokio::test]
async fn the_local_manager_reads_and_writes_nothing() {
    let world = World::new(instant(SHARED_AT));
    let manager = world.local_manager();
    assert!(manager.cached_health_check_results().await.is_none());
    manager.cache_health_check_results(&[], &[]).await;
    manager.release_health_check_lock().await;
    assert!(world.ops.entries().is_empty());
}
