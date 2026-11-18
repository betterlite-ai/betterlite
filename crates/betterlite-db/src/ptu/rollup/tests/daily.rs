use std::collections::BTreeSet;

use rust_decimal::Decimal;
use tracing::subscriber::set_default;

use super::support as stored;
use super::support::{
    active_info, clock, cost_of, daily, daily_with, day, ended_info, entry, group_of, models_on,
    no_database, row_count, store,
};
use crate::ptu::rollup::RollupError;
use crate::ptu::store::StoreError;

const DAY: &str = "2026-03-01";

#[tokio::test]
async fn the_day_defaults_to_yesterday_utc() {
    let (fixture, pool) = stored::database().await;
    let client = pool.get().await.unwrap();
    stored::insert_stored_model(&client, "model-a", "name-a", &active_info("team-1")).await;
    let result = daily(&store(&pool), &clock(), None, false).await.unwrap();
    assert_eq!(result.day, day("2026-02-28"));
    assert_eq!(result.models_processed, 1);
    assert_eq!(result.rows_written, 1);
    assert_eq!(result.rows_failed, 0);
    assert!(result.lapsed.is_empty());
    assert_eq!(
        cost_of(&client, "2026-02-28", "model-a"),
        Some(Decimal::from(24_i32))
    );
    stored::close(fixture, pool, client).await;
}

#[tokio::test]
async fn a_config_deployment_is_priced_alongside_the_stored_ones() {
    let (fixture, pool) = stored::database().await;
    let client = pool.get().await.unwrap();
    stored::insert_stored_model(&client, "db-a", "stored-a", &active_info("team-1")).await;
    let entries = [entry("cfg-b", "cfg-name", &active_info("team-2"))];
    let result = daily_with(&store(&pool), &entries, &clock(), Some(day(DAY)), true)
        .await
        .unwrap();
    assert_eq!(result.models_processed, 2);
    assert_eq!(result.rows_written, 2);
    assert_eq!(cost_of(&client, DAY, "db-a"), Some(Decimal::from(24_i32)));
    assert_eq!(cost_of(&client, DAY, "cfg-b"), Some(Decimal::from(24_i32)));
    assert_eq!(group_of(&client, DAY, "cfg-b").as_deref(), Some("cfg-name"));
    stored::close(fixture, pool, client).await;
}

#[tokio::test]
async fn a_missing_database_warns_and_reports_a_zero_count_day() {
    let (logs, subscriber) = stored::capture_logs();
    let guard = set_default(subscriber);
    let result = daily(&no_database(), &clock(), None, false).await.unwrap();
    drop(guard);
    assert_eq!(result.day, day("2026-02-28"));
    assert_eq!(result.models_processed, 0);
    assert_eq!(result.rows_written, 0);
    assert_eq!(result.rows_failed, 0);
    assert!(result.lapsed.is_empty());
    assert!(logs.contents().contains("no database is connected"));
}

#[tokio::test]
async fn a_failed_charge_keeps_the_landed_rows_and_skips_the_prune() {
    let (fixture, pool) = stored::database().await;
    let client = pool.get().await.unwrap();
    stored::insert_stored_model(&client, "model-a", "name-a", &active_info("team-1")).await;
    stored::insert_stored_model(&client, "model-b", "name-b", &active_info("team-1")).await;
    stored::insert_stored_model(
        &client,
        "model-z",
        "name-z",
        &ended_info("team-1", "2026-02-15T00:00:00Z"),
    )
    .await;
    stored::seed(
        &client,
        "('stale', 'team-1', '2026-03-01', '__ptu_flat_cost__', 'model-z', 'name-z', \
          '', '', '', 0, 0, 0, now(), '2026-03-01 11:00:00', 3.5)",
    )
    .await;
    stored::failing_upserts(&client, "model-a", 100).await;
    let (logs, subscriber) = stored::capture_logs();
    let guard = set_default(subscriber);
    let result = daily(&store(&pool), &clock(), Some(day(DAY)), true)
        .await
        .unwrap();
    drop(guard);
    assert_eq!(result.rows_written, 1);
    assert_eq!(result.rows_failed, 1);
    assert_eq!(
        cost_of(&client, DAY, "model-b"),
        Some(Decimal::from(24_i32))
    );
    assert_eq!(cost_of(&client, DAY, "model-z"), Some(Decimal::new(35, 1)));
    let captured = logs.contents();
    assert!(
        captured.contains("1 charge(s) failed for 2026-03-01"),
        "{captured}"
    );
    assert!(captured.contains("skipping the prune"), "{captured}");
    stored::close(fixture, pool, client).await;
}

#[tokio::test]
async fn a_scan_error_propagates() {
    let (fixture, pool) = stored::database().await;
    let client = pool.get().await.unwrap();
    stored::insert_stored_model(&client, "model-a", "name-a", &active_info("team-1")).await;
    client
        .batch_execute("DROP TABLE \"LiteLLM_ProxyModelTable\"")
        .await
        .unwrap();
    let result = daily(&store(&pool), &clock(), Some(day(DAY)), false).await;
    assert!(matches!(
        result,
        Err(RollupError::Store(StoreError::Scan(_)))
    ));
    stored::close(fixture, pool, client).await;
}

#[tokio::test]
async fn a_prune_error_propagates_after_the_writes_land() {
    let (fixture, pool) = stored::database().await;
    let client = pool.get().await.unwrap();
    stored::insert_stored_model(&client, "model-a", "name-a", &active_info("team-1")).await;
    stored::insert_stored_model(
        &client,
        "model-z",
        "name-z",
        &ended_info("team-1", "2026-02-15T00:00:00Z"),
    )
    .await;
    stored::seed(
        &client,
        "('stale', 'team-1', '2026-03-01', '__ptu_flat_cost__', 'model-z', 'name-z', \
          '', '', '', 0, 0, 0, now(), '2026-03-01 11:00:00', 3.5)",
    )
    .await;
    stored::rejecting_deletes(&client).await;
    let result = daily(&store(&pool), &clock(), Some(day(DAY)), true).await;
    assert!(matches!(
        result,
        Err(RollupError::Store(StoreError::Prune(_)))
    ));
    assert_eq!(
        cost_of(&client, DAY, "model-a"),
        Some(Decimal::from(24_i32))
    );
    assert_eq!(row_count(&client).await, 2);
    stored::close(fixture, pool, client).await;
}

#[tokio::test]
async fn an_unguarded_run_skips_the_prune_and_says_so() {
    let (fixture, pool) = stored::database().await;
    let client = pool.get().await.unwrap();
    stored::insert_stored_model(&client, "model-a", "name-a", &active_info("team-1")).await;
    stored::insert_stored_model(
        &client,
        "model-z",
        "name-z",
        &ended_info("team-1", "2026-02-15T00:00:00Z"),
    )
    .await;
    stored::seed(
        &client,
        "('stale', 'team-1', '2026-03-01', '__ptu_flat_cost__', 'model-z', 'name-z', \
          '', '', '', 0, 0, 0, now(), '2026-03-01 11:00:00', 3.5)",
    )
    .await;
    let (logs, subscriber) = stored::capture_logs();
    let guard = set_default(subscriber);
    let result = daily(&store(&pool), &clock(), Some(day(DAY)), false)
        .await
        .unwrap();
    drop(guard);
    assert_eq!(result.rows_written, 1);
    assert_eq!(cost_of(&client, DAY, "model-z"), Some(Decimal::new(35, 1)));
    assert!(
        logs.contents().contains("without the cross-pod lock"),
        "{}",
        logs.contents()
    );
    stored::close(fixture, pool, client).await;
}

#[tokio::test]
async fn the_prune_stays_inside_the_scanned_deployments() {
    let (fixture, pool) = stored::database().await;
    let client = pool.get().await.unwrap();
    stored::insert_stored_model(&client, "model-a", "name-a", &active_info("team-1")).await;
    stored::insert_stored_model(
        &client,
        "model-z",
        "name-z",
        &ended_info("team-1", "2026-02-15T00:00:00Z"),
    )
    .await;
    stored::seed(
        &client,
        "('stale', 'team-1', '2026-03-01', '__ptu_flat_cost__', 'model-z', 'name-z', \
          '', '', '', 0, 0, 0, now(), '2026-03-01 11:00:00', 3.5), \
         ('gone', 'team-1', '2026-03-01', '__ptu_flat_cost__', 'gone', 'gone', \
          '', '', '', 0, 0, 0, now(), '2026-03-01 11:00:00', 1.5)",
    )
    .await;
    daily(&store(&pool), &clock(), Some(day(DAY)), true)
        .await
        .unwrap();
    assert!(cost_of(&client, DAY, "model-z").is_none());
    let remaining: BTreeSet<String> = models_on(&client, DAY).into_iter().collect();
    assert_eq!(remaining, stored::names(&["gone", "model-a"]));
    stored::close(fixture, pool, client).await;
}

#[tokio::test]
async fn the_result_reports_lapsed_names_escaped_newest_first() {
    let (fixture, pool) = stored::database().await;
    let client = pool.get().await.unwrap();
    stored::insert_stored_model(
        &client,
        "model-a",
        "old<&>",
        &ended_info("team-1", "2026-02-20T00:00:00Z"),
    )
    .await;
    stored::insert_stored_model(
        &client,
        "model-b",
        "newer&",
        &ended_info("team-1", "2026-02-25T00:00:00Z"),
    )
    .await;
    let result = daily(&store(&pool), &clock(), Some(day(DAY)), false)
        .await
        .unwrap();
    assert_eq!(result.lapsed, ["newer&amp;", "old&lt;&amp;&gt;"]);
    stored::close(fixture, pool, client).await;
}
