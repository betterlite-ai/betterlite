use std::sync::Arc;

use chrono::NaiveDate;
use rust_decimal::Decimal;
use tracing::subscriber::set_default;

use super::support as stored;
use super::support::{
    AlertLog, FakeLease, FixedClock, Schedule, active_info, cacheless_manager, clock, cost_of, day,
    ended_info, manager, manager_that_cannot_acquire, ptu_info, row_count, scheduled,
    seed_lapsed_deployment, seed_sentinel, store,
};
use crate::postgres_store::session::PostgresStore;
use crate::ptu::rollup::RollupError;
use crate::queues::pod_lock::PodLockManager;

const DAY: &str = "2026-03-01";

fn schedule<'run>(
    store: &'run PostgresStore,
    clock: &'run FixedClock,
    lock: Option<&'run PodLockManager<FakeLease>>,
    alert: Option<&'run AlertLog>,
    target_date: Option<NaiveDate>,
) -> Schedule<'run> {
    Schedule {
        store,
        clock,
        lock,
        alert,
        attribution_enabled: true,
        target_date,
    }
}

#[tokio::test]
async fn a_disabled_flag_returns_none_before_the_database() {
    let (fixture, pool) = stored::database().await;
    pool.close();
    let postgres = store(&pool);
    let disabled = scheduled(Schedule {
        attribution_enabled: false,
        ..schedule(&postgres, &clock(), None, None, Some(day(DAY)))
    })
    .await
    .unwrap();
    assert!(disabled.is_none());
    let enabled = scheduled(schedule(&postgres, &clock(), None, None, Some(day(DAY)))).await;
    assert!(matches!(enabled, Err(RollupError::Session(_))));
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn a_run_without_a_lock_manager_runs_unguarded() {
    let (fixture, pool) = stored::database().await;
    let client = pool.get().await.unwrap();
    stored::insert_stored_model(&client, "model-a", "name-a", &active_info("team-1")).await;
    seed_lapsed_deployment(&client, DAY).await;
    let postgres = store(&pool);
    let (logs, subscriber) = stored::capture_logs();
    let guard = set_default(subscriber);
    let unguarded = scheduled(schedule(&postgres, &clock(), None, None, Some(day(DAY))))
        .await
        .unwrap();
    let uncached_manager = cacheless_manager();
    let uncached = scheduled(schedule(
        &postgres,
        &clock(),
        Some(&uncached_manager),
        None,
        Some(day(DAY)),
    ))
    .await
    .unwrap();
    drop(guard);
    assert!(unguarded.is_some());
    assert!(uncached.is_some());
    assert_eq!(cost_of(&client, DAY, "model-z"), Some(Decimal::new(35, 1)));
    assert!(logs.contents().contains("without the cross-pod lock"));
    stored::close(fixture, pool, client).await;
}

#[tokio::test]
async fn a_held_lock_skips_the_run() {
    let (fixture, pool) = stored::database().await;
    let client = pool.get().await.unwrap();
    stored::insert_stored_model(&client, "model-a", "name-a", &active_info("team-1")).await;
    let lease = Arc::new(FakeLease::held_by("other-pod"));
    let held = manager(&lease);
    let postgres = store(&pool);
    let (logs, subscriber) = stored::capture_logs();
    let guard = set_default(subscriber);
    let result = scheduled(schedule(
        &postgres,
        &clock(),
        Some(&held),
        None,
        Some(day(DAY)),
    ))
    .await
    .unwrap();
    drop(guard);
    assert!(result.is_none());
    assert_eq!(row_count(&client).await, 0);
    assert_eq!(lease.deletes(), 0);
    assert!(
        logs.contents()
            .contains("another pod holds the rollup lock")
    );
    stored::close(fixture, pool, client).await;
}

#[tokio::test]
async fn a_failed_acquire_without_a_visible_holder_runs_unguarded() {
    let (fixture, pool) = stored::database().await;
    let client = pool.get().await.unwrap();
    stored::insert_stored_model(&client, "model-a", "name-a", &active_info("team-1")).await;
    seed_lapsed_deployment(&client, DAY).await;
    let manager = manager_that_cannot_acquire(false);
    let postgres = store(&pool);
    let (logs, subscriber) = stored::capture_logs();
    let guard = set_default(subscriber);
    let absent_run = scheduled(schedule(
        &postgres,
        &clock(),
        Some(&manager),
        None,
        Some(day(DAY)),
    ))
    .await
    .unwrap();
    drop(guard);
    assert!(absent_run.is_some());
    assert_eq!(cost_of(&client, DAY, "model-z"), Some(Decimal::new(35, 1)));
    assert!(
        logs.contents().contains("no other pod holds it"),
        "{}",
        logs.contents()
    );
    stored::close(fixture, pool, client).await;
}

#[tokio::test]
async fn an_unreadable_lock_runs_unguarded_after_the_warning() {
    let (fixture, pool) = stored::database().await;
    let client = pool.get().await.unwrap();
    stored::insert_stored_model(&client, "model-a", "name-a", &active_info("team-1")).await;
    seed_lapsed_deployment(&client, DAY).await;
    let manager = manager_that_cannot_acquire(true);
    let postgres = store(&pool);
    let (logs, subscriber) = stored::capture_logs();
    let guard = set_default(subscriber);
    let unreadable_run = scheduled(schedule(
        &postgres,
        &clock(),
        Some(&manager),
        None,
        Some(day(DAY)),
    ))
    .await
    .unwrap();
    drop(guard);
    assert!(unreadable_run.is_some());
    assert_eq!(cost_of(&client, DAY, "model-z"), Some(Decimal::new(35, 1)));
    assert!(
        logs.contents().contains("could not read the rollup lock"),
        "{}",
        logs.contents()
    );
    stored::close(fixture, pool, client).await;
}

#[tokio::test]
async fn an_acquired_lock_prunes_and_is_released() {
    let (fixture, pool) = stored::database().await;
    let client = pool.get().await.unwrap();
    stored::insert_stored_model(&client, "model-a", "name-a", &active_info("team-1")).await;
    seed_lapsed_deployment(&client, DAY).await;
    seed_sentinel(&client, DAY, "gone", "1.5").await;
    let lease = Arc::new(FakeLease::default());
    let owned = manager(&lease);
    let result = scheduled(schedule(
        &store(&pool),
        &clock(),
        Some(&owned),
        None,
        Some(day(DAY)),
    ))
    .await
    .unwrap();
    assert!(result.is_some());
    assert!(cost_of(&client, DAY, "model-z").is_none());
    assert_eq!(cost_of(&client, DAY, "gone"), Some(Decimal::new(15, 1)));
    assert_eq!(lease.deletes(), 1);
    assert!(lease.owner.lock().unwrap().is_none());
    stored::close(fixture, pool, client).await;
}

#[tokio::test]
async fn a_failed_charge_alerts_before_the_lapsed_alert_with_at_most_ten_names() {
    let (fixture, pool) = stored::database().await;
    let client = pool.get().await.unwrap();
    stored::insert_stored_model(&client, "model-a", "name-a", &active_info("team-1")).await;
    stored::failing_upserts(&client, "model-a", 100).await;
    for index in 0..12_u32 {
        let name = format!("lapsed-{index}");
        stored::insert_stored_model(
            &client,
            &name,
            &name,
            &ended_info("team-1", "2026-02-15T00:00:00Z"),
        )
        .await;
    }
    let owned = manager(&Arc::new(FakeLease::default()));
    let alert = AlertLog::default();
    let result = scheduled(schedule(
        &store(&pool),
        &clock(),
        Some(&owned),
        Some(&alert),
        Some(day(DAY)),
    ))
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result.rows_failed, 1);
    assert_eq!(result.lapsed.len(), 12);
    let messages = alert.messages();
    assert_eq!(messages.len(), 2);
    assert!(messages[0].contains("failed to write"));
    assert!(messages[0].contains("rerun"));
    assert!(messages[1].contains("attribution has stopped"));
    assert!(messages[1].contains("12 deployment(s)"));
    assert_eq!(messages[1].matches("lapsed-").count(), 10);
    stored::close(fixture, pool, client).await;
}

#[tokio::test]
async fn an_unavailable_or_absent_alert_channel_never_fails_the_run() {
    let (fixture, pool) = stored::database().await;
    let client = pool.get().await.unwrap();
    stored::insert_stored_model(&client, "model-a", "name-a", &active_info("team-1")).await;
    stored::failing_upserts(&client, "model-a", 100).await;
    let down = AlertLog {
        down: true,
        ..AlertLog::default()
    };
    let postgres = store(&pool);
    let (logs, subscriber) = stored::capture_logs();
    let guard = set_default(subscriber);
    let with_channel = scheduled(schedule(
        &postgres,
        &clock(),
        None,
        Some(&down),
        Some(day(DAY)),
    ))
    .await
    .unwrap()
    .unwrap();
    let without_channel = scheduled(schedule(&postgres, &clock(), None, None, Some(day(DAY))))
        .await
        .unwrap()
        .unwrap();
    drop(guard);
    assert_eq!(with_channel.rows_failed, 1);
    assert_eq!(without_channel.rows_failed, 1);
    assert!(
        logs.contents()
            .contains("could not deliver the failed-charge alert")
    );
    stored::close(fixture, pool, client).await;
}

#[tokio::test]
async fn catch_up_runs_only_when_the_target_date_is_absent() {
    let (fixture, pool) = stored::database().await;
    let client = pool.get().await.unwrap();
    stored::insert_stored_model(
        &client,
        "model-a",
        "name-a",
        &ptu_info("team-1", 1, "1", "2026-02-26T00:00:00Z", None),
    )
    .await;
    let targeted = scheduled(schedule(
        &store(&pool),
        &clock(),
        None,
        None,
        Some(day("2026-02-28")),
    ))
    .await
    .unwrap()
    .unwrap();
    assert_eq!(targeted.day, day("2026-02-28"));
    assert!(cost_of(&client, "2026-02-26", "model-a").is_none());
    assert!(cost_of(&client, "2026-02-27", "model-a").is_none());
    assert_eq!(row_count(&client).await, 1);
    stored::close(fixture, pool, client).await;
}

#[tokio::test]
async fn an_absent_target_date_catches_up_the_missing_days() {
    let (fixture, pool) = stored::database().await;
    let client = pool.get().await.unwrap();
    stored::insert_stored_model(
        &client,
        "model-a",
        "name-a",
        &ptu_info("team-1", 1, "1", "2026-02-26T00:00:00Z", None),
    )
    .await;
    let alert = AlertLog::default();
    let full = scheduled(schedule(&store(&pool), &clock(), None, Some(&alert), None))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(full.day, day("2026-02-28"));
    assert_eq!(
        cost_of(&client, "2026-02-26", "model-a"),
        Some(Decimal::from(24_i32))
    );
    assert_eq!(
        cost_of(&client, "2026-02-27", "model-a"),
        Some(Decimal::from(24_i32))
    );
    assert_eq!(row_count(&client).await, 3);
    assert!(alert.messages().is_empty());
    stored::close(fixture, pool, client).await;
}

#[tokio::test]
async fn a_catch_up_failure_is_logged_and_the_daily_result_survives() {
    let (fixture, pool) = stored::database().await;
    let client = pool.get().await.unwrap();
    stored::insert_stored_model(&client, "model-a", "name-a", &active_info("team-1")).await;
    client
        .batch_execute("ALTER TABLE \"LiteLLM_DailyTeamSpend\" RENAME COLUMN \"date\" TO \"day\"")
        .await
        .unwrap();
    let alert = AlertLog::default();
    let postgres = store(&pool);
    let (logs, subscriber) = stored::capture_logs();
    let guard = set_default(subscriber);
    let result = scheduled(schedule(&postgres, &clock(), None, Some(&alert), None))
        .await
        .unwrap()
        .unwrap();
    drop(guard);
    assert_eq!(result.day, day("2026-02-28"));
    assert_eq!(result.rows_failed, 1);
    assert!(
        logs.contents().contains("the day's rollup still stands"),
        "{}",
        logs.contents()
    );
    assert_eq!(alert.messages().len(), 1);
    stored::close(fixture, pool, client).await;
}

#[tokio::test]
async fn a_catch_up_charge_failure_alerts_the_unpriced_days() {
    let (fixture, pool) = stored::database().await;
    let client = pool.get().await.unwrap();
    stored::insert_stored_model(
        &client,
        "model-a",
        "name-a",
        &ptu_info("team-1", 1, "1", "2026-02-26T00:00:00Z", None),
    )
    .await;
    stored::failing_upserts_on_date(&client, "2026-02-27").await;
    let alert = AlertLog::default();
    let result = scheduled(schedule(&store(&pool), &clock(), None, Some(&alert), None))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.rows_failed, 0);
    assert_eq!(
        cost_of(&client, "2026-02-26", "model-a"),
        Some(Decimal::from(24_i32))
    );
    assert!(cost_of(&client, "2026-02-27", "model-a").is_none());
    assert_eq!(
        cost_of(&client, "2026-02-28", "model-a"),
        Some(Decimal::from(24_i32))
    );
    let messages = alert.messages();
    assert_eq!(messages.len(), 1);
    assert!(messages[0].contains("PTU flat-cost backfill for 2026-02-26 to 2026-02-28"));
    assert!(messages[0].contains("1 of 2"));
    stored::close(fixture, pool, client).await;
}
