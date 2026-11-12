use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Notify;
use tokio::task::{JoinSet, yield_now};
use tokio::time::{advance, timeout};
use tracing::subscriber::set_default;

use crate::database_refresh::access;
use crate::database_refresh::endpoint::DatabaseUrlSlot;
use crate::database_refresh::handle::PoolHandle as _;
use crate::database_refresh::handle::PoolTransport;
use crate::database_refresh::replacement::{self, ReplacementError};
use crate::database_refresh::retirement::{self, wait_for_pool_replacement};

use super::support::{
    ConnectStep, FakeScript, NEXT_URL, callback, capture_logs, create, failing_callback, options,
    retirement_deadline, secret, set_url, stop, wait_for_event,
};

#[tokio::test]
async fn stale_generation_no_effects() {
    let script = Arc::new(FakeScript::default());
    let slot = DatabaseUrlSlot::default();
    set_url(&slot, NEXT_URL);
    let built = create(&script, options(slot), None, None, true).await;
    let (logs, subscriber) = capture_logs();
    let guard = set_default(subscriber);
    let replaced = replacement::recreate(&built, &secret(NEXT_URL), None, Some(7))
        .await
        .expect("a mismatched recreate returns");
    drop(guard);
    assert!(!replaced);
    assert_eq!(built.observe().generation, 0);
    assert_eq!(
        script.events(),
        ["build:0".to_owned(), "connect:0".to_owned()]
    );
    let captured = logs.contents();
    assert!(captured.contains("current 0"), "{captured}");
    assert!(captured.contains("expected 7"), "{captured}");
}

#[tokio::test]
async fn competing_recoveries_coalesce() {
    let script = Arc::new(FakeScript::default());
    let gate = Arc::new(Notify::new());
    script.script_connect(1, ConnectStep::Blocked);
    script.gate_connect(1, Arc::clone(&gate));
    let slot = DatabaseUrlSlot::default();
    set_url(&slot, NEXT_URL);
    let built = create(&script, options(slot), None, None, true).await;
    let mut tasks = JoinSet::new();
    let first_generation = Arc::clone(&built);
    let first_url = secret(NEXT_URL);
    tasks.spawn(async move {
        replacement::recreate(&first_generation, &first_url, None, Some(0)).await
    });
    wait_for_event(&script, "connect:1").await;
    let second_generation = Arc::clone(&built);
    let second_url = secret(NEXT_URL);
    tasks.spawn(async move {
        replacement::recreate(&second_generation, &second_url, None, Some(0)).await
    });
    yield_now().await;
    gate.notify_one();
    let first = tasks
        .join_next()
        .await
        .expect("the first recovery finishes")
        .expect("the first task stays healthy");
    let second = tasks
        .join_next()
        .await
        .expect("the second recovery finishes")
        .expect("the second task stays healthy");
    assert!(first.expect("the first recovery wins"));
    assert!(!second.expect("the second recovery declines"));
    assert_eq!(script.build_urls().len(), 2);
    assert_eq!(built.observe().generation, 1);
}

#[tokio::test]
async fn recovery_failure_keeps_counter() {
    let script = Arc::new(FakeScript::default());
    script.script_connect(1, ConnectStep::Fail);
    let slot = DatabaseUrlSlot::default();
    set_url(&slot, NEXT_URL);
    let built = create(&script, options(slot), None, None, true).await;
    let initial = built.observe().handle.expect("the initial handle");
    let outcome = replacement::recreate(&built, &secret(NEXT_URL), None, None).await;
    assert!(matches!(outcome, Err(ReplacementError::Connect { .. })));
    let observed = built.observe();
    assert_eq!(observed.generation, 0);
    assert_eq!(observed.handle.expect("the replacement handle").id, 1);
    assert!(!observed.connected);
    assert!(!initial.is_connected());
    assert!(script.has_event("close:0"));
}

#[tokio::test]
async fn refresh_cancel_cleans_candidate() {
    let script = Arc::new(FakeScript::default());
    let gate = Arc::new(Notify::new());
    script.script_connect(1, ConnectStep::Blocked);
    script.gate_connect(1, Arc::clone(&gate));
    let slot = DatabaseUrlSlot::default();
    let built = create(&script, options(slot.clone()), None, None, true).await;
    let initial = built.observe().handle.expect("the initial handle");
    set_url(&slot, NEXT_URL);
    let mut tasks = JoinSet::new();
    let replacing = Arc::clone(&built);
    let url = secret(NEXT_URL);
    tasks.spawn(async move {
        let _lock = replacing.replacement_lock.lock().await;
        replacement::replace_locked(&replacing, &url, None).await
    });
    wait_for_event(&script, "connect:1").await;
    stop(&mut tasks).await;
    assert!(script.has_event("close:1"));
    assert!(!script.has_event("close:0"));
    let observed = built.observe();
    assert_eq!(observed.generation, 0);
    assert!(observed.connected);
    assert!(Arc::ptr_eq(
        &observed.handle.expect("the old handle"),
        &initial
    ));
    assert!(initial.is_connected());
}

#[tokio::test]
async fn refresh_publish_then_callback_then_retire() {
    let script = Arc::new(FakeScript::default());
    let slot = DatabaseUrlSlot::default();
    let built = create(
        &script,
        options(slot.clone()),
        None,
        Some(callback(&script)),
        true,
    )
    .await;
    let initial = built.observe().handle.expect("the initial handle");
    set_url(&slot, NEXT_URL);
    let lock_guard = built.replacement_lock.lock().await;
    replacement::replace_locked(&built, &secret(NEXT_URL), None)
        .await
        .expect("the candidate replaces");
    drop(lock_guard);
    yield_now().await;
    yield_now().await;
    let callback_at = script.event_position("callback").expect("the callback ran");
    let close_at = script
        .event_position("close:0")
        .expect("the old pool closed");
    assert!(callback_at < close_at);
    assert_eq!(built.observe().generation, 1);
    assert_eq!(built.observe().handle.expect("the new handle").id, 1);
    assert!(!initial.is_connected());
}

#[tokio::test]
async fn callback_error_after_publication() {
    let script = Arc::new(FakeScript::default());
    let slot = DatabaseUrlSlot::default();
    let built = create(
        &script,
        options(slot.clone()),
        None,
        Some(failing_callback()),
        true,
    )
    .await;
    set_url(&slot, NEXT_URL);
    let lock_guard = built.replacement_lock.lock().await;
    let outcome = replacement::replace_locked(&built, &secret(NEXT_URL), None).await;
    drop(lock_guard);
    assert!(matches!(outcome, Err(ReplacementError::Callback { .. })));
    let observed = built.observe();
    assert_eq!(observed.generation, 1);
    assert_eq!(observed.handle.expect("the published handle").id, 1);
    assert!(observed.connected);
    yield_now().await;
    assert!(!script.has_event("close:0"));
}

#[tokio::test]
async fn writer_snapshot_vs_explicit_reader_url() {
    let script = Arc::new(FakeScript::default());
    let slot = DatabaseUrlSlot::default();
    set_url(&slot, "postgres://snapshot");
    let writer = create(&script, options(slot.clone()), None, None, true).await;
    replacement::recreate(&writer, &secret("postgres://passed"), None, None)
        .await
        .expect("the writer replaces");
    assert_eq!(
        script.build_urls().last().expect("a build"),
        "postgres://snapshot"
    );
    set_url(&slot, "postgres://published");
    replacement::recreate(&writer, &secret("postgres://ignored"), None, None)
        .await
        .expect("the writer replaces again");
    assert_eq!(
        script.build_urls().last().expect("a build"),
        "postgres://published"
    );
    assert!(
        !script
            .build_urls()
            .contains(&"postgres://passed".to_owned())
    );
    let reader_slot = DatabaseUrlSlot::default();
    let mut reader_settings = options(reader_slot);
    reader_settings.recreate_uses_datasource = true;
    let reader = create(&script, reader_settings, None, None, true).await;
    replacement::recreate(&reader, &secret("postgres://reader"), None, None)
        .await
        .expect("the reader replaces");
    assert_eq!(
        script.build_urls().last().expect("a build"),
        "postgres://reader"
    );
}

#[tokio::test]
async fn retirement_waits_for_transaction() {
    let script = Arc::new(FakeScript::default());
    let slot = DatabaseUrlSlot::default();
    let built = create(&script, options(slot.clone()), None, None, true).await;
    let handle = access::operation(&built).expect("a connected operation");
    let tracker = handle.tracker.clone().expect("a tracker");
    let held = tracker.begin_transaction();
    set_url(&slot, NEXT_URL);
    let lock_guard = built.replacement_lock.lock().await;
    replacement::replace_locked(&built, &secret(NEXT_URL), None)
        .await
        .expect("the candidate replaces");
    drop(lock_guard);
    yield_now().await;
    yield_now().await;
    assert!(!script.has_event("close:0"));
    drop(held);
    yield_now().await;
    yield_now().await;
    assert!(script.has_event("close:0"));
}

#[tokio::test]
async fn missing_tracker_retires_immediately() {
    let script = Arc::new(FakeScript::default());
    let slot = DatabaseUrlSlot::default();
    let built = create(&script, options(slot.clone()), None, None, false).await;
    set_url(&slot, NEXT_URL);
    let lock_guard = built.replacement_lock.lock().await;
    replacement::replace_locked(&built, &secret(NEXT_URL), None)
        .await
        .expect("the candidate replaces");
    drop(lock_guard);
    yield_now().await;
    yield_now().await;
    assert_eq!(built.observe().generation, 1);
    assert!(script.has_event("close:0"));
    assert_eq!(retirement::retirement_count(&built), 0);
}

#[tokio::test]
async fn completed_retirement_removed() {
    let script = Arc::new(FakeScript::default());
    let slot = DatabaseUrlSlot::default();
    let built = create(&script, options(slot.clone()), None, None, true).await;
    set_url(&slot, NEXT_URL);
    let lock_guard = built.replacement_lock.lock().await;
    replacement::replace_locked(&built, &secret(NEXT_URL), None)
        .await
        .expect("the candidate replaces");
    drop(lock_guard);
    yield_now().await;
    yield_now().await;
    assert!(script.has_event("close:0"));
    assert_eq!(retirement::retirement_count(&built), 0);
}

#[tokio::test]
async fn shutdown_releases_retirements() {
    let script = Arc::new(FakeScript::default());
    let slot = DatabaseUrlSlot::default();
    let built = create(&script, options(slot.clone()), None, None, true).await;
    let handle = access::operation(&built).expect("a connected operation");
    let tracker = handle.tracker.clone().expect("a tracker");
    let held = tracker.begin_transaction();
    set_url(&slot, NEXT_URL);
    let lock_guard = built.replacement_lock.lock().await;
    replacement::replace_locked(&built, &secret(NEXT_URL), None)
        .await
        .expect("the candidate replaces");
    drop(lock_guard);
    yield_now().await;
    assert_eq!(retirement::retirement_count(&built), 1);
    retirement::shutdown(&built).await;
    assert_eq!(retirement::retirement_count(&built), 0);
    drop(held);
}

#[tokio::test]
async fn wait_without_lock_returns() {
    let script = Arc::new(FakeScript::default());
    let built = create(
        &script,
        options(DatabaseUrlSlot::default()),
        None,
        None,
        true,
    )
    .await;
    timeout(
        Duration::from_secs(1),
        wait_for_pool_replacement(&built, Duration::from_secs(300)),
    )
    .await
    .expect("a free lock returns at once");
}

#[tokio::test]
async fn nonpositive_wait_returns() {
    let script = Arc::new(FakeScript::default());
    let built = create(
        &script,
        options(DatabaseUrlSlot::default()),
        None,
        None,
        true,
    )
    .await;
    let lock_guard = built.replacement_lock.lock().await;
    timeout(
        Duration::from_millis(50),
        wait_for_pool_replacement(&built, Duration::ZERO),
    )
    .await
    .expect("a nonpositive wait returns at once");
    drop(lock_guard);
}

#[tokio::test(start_paused = true)]
async fn wait_timeout_is_not_success() {
    let script = Arc::new(FakeScript::default());
    let built = create(
        &script,
        options(DatabaseUrlSlot::default()),
        None,
        None,
        true,
    )
    .await;
    let gate = Arc::new(Notify::new());
    let mut holder_tasks = JoinSet::new();
    let holding = Arc::clone(&built);
    let holder_gate = Arc::clone(&gate);
    holder_tasks.spawn(async move {
        let _lock = holding.replacement_lock.lock().await;
        holder_gate.notified().await;
    });
    yield_now().await;
    let mut waiters = JoinSet::new();
    let waiting = Arc::clone(&built);
    waiters.spawn(async move {
        wait_for_pool_replacement(&waiting, Duration::from_secs(90)).await;
    });
    yield_now().await;
    assert!(!waiters.is_empty());
    advance(Duration::from_secs(90)).await;
    waiters
        .join_next()
        .await
        .expect("the waiter settles")
        .expect("the waiter task stays healthy");
    assert_eq!(built.observe().generation, 0);
    stop(&mut holder_tasks).await;
}

#[tokio::test]
async fn failed_replace_has_no_generation_change() {
    let script = Arc::new(FakeScript::default());
    script.script_connect(1, ConnectStep::Fail);
    let slot = DatabaseUrlSlot::default();
    let built = create(&script, options(slot.clone()), None, None, true).await;
    set_url(&slot, NEXT_URL);
    let lock_guard = built.replacement_lock.lock().await;
    let outcome = replacement::replace_locked(&built, &secret(NEXT_URL), None).await;
    drop(lock_guard);
    assert!(matches!(outcome, Err(ReplacementError::Connect { .. })));
    assert_eq!(built.observe().generation, 0);
    assert!(built.observe().connected);
}

#[tokio::test]
async fn waiter_cancel_does_not_cancel_replace() {
    let script = Arc::new(FakeScript::default());
    let gate = Arc::new(Notify::new());
    script.script_connect(1, ConnectStep::Blocked);
    script.gate_connect(1, Arc::clone(&gate));
    let slot = DatabaseUrlSlot::default();
    set_url(&slot, NEXT_URL);
    let built = create(&script, options(slot), None, None, true).await;
    let mut replacers = JoinSet::new();
    let replacing = Arc::clone(&built);
    let url = secret(NEXT_URL);
    replacers.spawn(async move { replacement::recreate(&replacing, &url, None, Some(0)).await });
    wait_for_event(&script, "connect:1").await;
    let mut waiters = JoinSet::new();
    let waiting = Arc::clone(&built);
    waiters.spawn(async move {
        wait_for_pool_replacement(&waiting, Duration::from_secs(60)).await;
    });
    yield_now().await;
    stop(&mut waiters).await;
    gate.notify_one();
    let outcome = replacers
        .join_next()
        .await
        .expect("the replacement finishes")
        .expect("the replacement task stays healthy");
    assert!(outcome.expect("the replacement completes"));
    assert_eq!(built.observe().generation, 1);
}

#[tokio::test]
async fn transport_override_forwarded_when_present() {
    let script = Arc::new(FakeScript::default());
    let slot = DatabaseUrlSlot::default();
    set_url(&slot, NEXT_URL);
    let built = create(&script, options(slot), None, None, true).await;
    let transport = PoolTransport { max_size: Some(5) };
    replacement::recreate(&built, &secret(NEXT_URL), Some(&transport), None)
        .await
        .expect("the tuned replacement");
    replacement::recreate(&built, &secret(NEXT_URL), None, None)
        .await
        .expect("the plain replacement");
    assert_eq!(script.transports(), vec![None, Some(5), None]);
}

#[tokio::test(start_paused = true)]
async fn retirement_timeout_warns_and_closes_old_only() {
    let deadline = retirement_deadline().await;
    assert!(
        deadline.logs.contains("did not drain within 90s"),
        "{}",
        deadline.logs
    );
    assert!(
        deadline.logs.contains("work still in flight"),
        "{}",
        deadline.logs
    );
    assert!(deadline.script.has_event("close:0"));
    assert!(!deadline.script.has_event("close:1"));
    let operations = access::operation(&deadline.built).expect("a connected operation");
    assert_eq!(
        operations.query("new-work").await.expect("new work runs"),
        0
    );
}

#[tokio::test(start_paused = true)]
async fn native_generation_logs_without_process_ids() {
    let deadline = retirement_deadline().await;
    assert!(!deadline.logs.contains("PID"), "{}", deadline.logs);
    assert!(deadline.logs.contains("generation 0"), "{}", deadline.logs);
}
