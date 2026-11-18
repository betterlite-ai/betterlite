use rust_decimal::Decimal;
use tokio_postgres::types::Json;
use tracing::subscriber::set_default;

use super::support as stored;
use super::support::{backfill, cell, clock, cost_of, day, no_database, ptu_info, store};
use crate::ptu::store::sentinel::write_charge_with_retry;

#[tokio::test]
async fn a_missing_database_reports_start_equal_to_end() {
    let (logs, subscriber) = stored::capture_logs();
    let guard = set_default(subscriber);
    let result = backfill(&no_database(), &clock(), None).await.unwrap();
    drop(guard);
    assert_eq!(result.start, day("2026-02-28"));
    assert_eq!(result.end, day("2026-02-28"));
    assert_eq!(result.days_scanned, 0);
    assert_eq!(result.rows_written, 0);
    assert_eq!(result.rows_failed, 0);
    assert!(logs.contents().contains("no database is connected"));
}

#[tokio::test]
async fn an_empty_catalog_reports_start_equal_to_end() {
    let (fixture, pool) = stored::database().await;
    let client = pool.get().await.unwrap();
    let result = backfill(&store(&pool), &clock(), None).await.unwrap();
    assert_eq!(result.start, day("2026-02-28"));
    assert_eq!(result.end, day("2026-02-28"));
    assert_eq!(result.days_scanned, 0);
    let explicit = backfill(&store(&pool), &clock(), Some(day("2026-01-10")))
        .await
        .unwrap();
    assert_eq!(explicit.start, day("2026-01-09"));
    assert_eq!(explicit.end, day("2026-01-09"));
    assert_eq!(explicit.days_scanned, 0);
    stored::close(fixture, pool, client).await;
}

#[tokio::test]
async fn an_existing_sentinel_keeps_its_billed_amount_after_a_rate_change() {
    let (fixture, pool) = stored::database().await;
    let client = pool.get().await.unwrap();
    stored::insert_stored_model(
        &client,
        "model-a",
        "name-a",
        &ptu_info("team-1", 1, "1", "2026-02-26T00:00:00Z", None),
    )
    .await;
    let first = backfill(&store(&pool), &clock(), None).await.unwrap();
    assert_eq!(first.start, day("2026-02-26"));
    assert_eq!(first.end, day("2026-02-28"));
    assert_eq!(first.days_scanned, 3);
    assert_eq!(first.rows_written, 3);
    let raised = ptu_info("team-1", 1, "2", "2026-02-26T00:00:00Z", None);
    client
        .execute(
            "UPDATE \"LiteLLM_ProxyModelTable\" SET \"model_info\" = $1, \
             \"model_name\" = 'renamed' WHERE \"model_id\" = 'model-a'",
            &[&Json(&raised)],
        )
        .await
        .unwrap();
    let second = backfill(&store(&pool), &clock(), None).await.unwrap();
    assert_eq!(second.days_scanned, 3);
    assert_eq!(second.rows_written, 0);
    assert_eq!(second.rows_failed, 0);
    assert_eq!(
        cost_of(&client, "2026-02-28", "model-a").await,
        Some(Decimal::from(24_i32))
    );
    assert_eq!(
        cell(&client, "model_group", "2026-02-28", "model-a")
            .await
            .as_deref(),
        Some("name-a")
    );
    stored::close(fixture, pool, client).await;
}

#[tokio::test]
async fn only_missing_keys_are_written_in_oldest_day_then_model_order() {
    let (fixture, pool) = stored::database().await;
    let client = pool.get().await.unwrap();
    let info = ptu_info("team-1", 1, "1", "2026-02-26T00:00:00Z", None);
    stored::insert_stored_model(&client, "model-a", "name-a", &info).await;
    stored::insert_stored_model(&client, "model-b", "name-b", &info).await;
    let kept = stored::charge("team-1", "model-a", "kept", Decimal::new(75, 1));
    let mut sleeper = stored::Recorder::default();
    let write = stored::write_for(&kept, "2026-02-27");
    assert!(write_charge_with_retry(&client, &mut sleeper, &write).await);
    stored::ddl(
        &client,
        "CREATE TABLE ptu_write_log (seq serial, model text, date text); \
         CREATE FUNCTION ptu_log_write() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN INSERT INTO ptu_write_log(model, date) VALUES (NEW.model, NEW.date); RETURN NEW; END; \
         $$; \
         CREATE TRIGGER ptu_log_write_trigger BEFORE INSERT OR UPDATE \
         ON \"LiteLLM_DailyTeamSpend\" FOR EACH ROW EXECUTE FUNCTION ptu_log_write();",
    )
    .await;
    let result = backfill(&store(&pool), &clock(), None).await.unwrap();
    assert_eq!(result.days_scanned, 3);
    assert_eq!(result.rows_written, 5);
    assert_eq!(result.rows_failed, 0);
    assert_eq!(
        stored::logged_writes(&client).await,
        [
            "model-a|2026-02-26",
            "model-b|2026-02-26",
            "model-b|2026-02-27",
            "model-a|2026-02-28",
            "model-b|2026-02-28",
        ]
    );
    assert_eq!(
        cost_of(&client, "2026-02-27", "model-a").await,
        Some(Decimal::new(75, 1))
    );
    stored::close(fixture, pool, client).await;
}

#[tokio::test]
async fn a_failed_charge_keeps_the_other_missing_rows_and_counts() {
    let (fixture, pool) = stored::database().await;
    let client = pool.get().await.unwrap();
    let info = ptu_info("team-1", 1, "1", "2026-02-28T00:00:00Z", None);
    stored::insert_stored_model(&client, "model-a", "name-a", &info).await;
    stored::insert_stored_model(&client, "model-b", "name-b", &info).await;
    stored::reject_writes(&client, "model", "model-a").await;
    let result = backfill(&store(&pool), &clock(), None).await.unwrap();
    assert_eq!(result.days_scanned, 1);
    assert_eq!(result.rows_written, 1);
    assert_eq!(result.rows_failed, 1);
    assert!(cost_of(&client, "2026-02-28", "model-a").await.is_none());
    assert_eq!(
        cost_of(&client, "2026-02-28", "model-b").await,
        Some(Decimal::from(24_i32))
    );
    stored::close(fixture, pool, client).await;
}
