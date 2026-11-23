#![cfg(test)]

use std::time::{Duration, Instant};

use betterlite_models::baseline_history::model::BaselineHistory;
use chrono::{TimeZone as _, Utc};
use deadpool_postgres::Client;
use rstest::rstest;

use self::support::{
    OTHER_SCOPE, SCOPE, comparison, harness, observation, reason, record, seed_logs, session, store,
};
use super::error::StoreError;
use super::retire::retire_before;
use super::{AppendResult, BaselineAccounting as _, ProjectResult};
use crate::baseline::publication::model::PublicationStatus;
use crate::postgres_store::session::PostgresStore;

pub(crate) mod support;

const STAMP_ATTEMPT: &str =
    r#"UPDATE "LiteLLM_AutoRouterBaselineComparison" SET attempted_at = CURRENT_TIMESTAMP"#;
const RETIRE_SCOPE: &str = r#"UPDATE "LiteLLM_AutoRouterBaselineComparison" SET retired = TRUE WHERE scope = 'autorouter-baseline:v3:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef'"#;
const AGE_SCOPE: &str = r#"UPDATE "LiteLLM_AutoRouterBaselineComparison" SET updated_at = '2000-01-01' WHERE scope = 'autorouter-baseline:v3:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef'"#;
const LOCK_SCOPE: &str = r#"BEGIN; SELECT 1 FROM "LiteLLM_AutoRouterBaselineComparison" WHERE scope = 'autorouter-baseline:v3:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef' FOR UPDATE"#;
const SERIES_LOGS: &str = r#"INSERT INTO "LiteLLM_SpendLogs" (request_id, call_type, api_key, "startTime", "endTime", metadata) SELECT 'series-' || g, 'acompletion', 'key', NOW(), NOW(), '{}'::jsonb FROM generate_series(1, 130) AS g"#;
const SERIES_IDENTICAL: &str = r#"SELECT COUNT(*) FROM "LiteLLM_AutoRouterBaselineObservation" WHERE publication::jsonb->>'reason' = 'identical_baseline_path'"#;
const PUBLISHED_ROWS: &str =
    r#"SELECT COUNT(*) FROM "LiteLLM_AutoRouterBaselineObservation" WHERE publication IS NOT NULL"#;
const ESTIMATED_LOGS: &str =
    r#"SELECT COUNT(*) FROM "LiteLLM_SpendLogs" WHERE metadata ? 'autorouter_savings_estimate'"#;
const FAIL_LAST_PAGE: &str = "CREATE FUNCTION fail_last_page() RETURNS trigger AS $$ BEGIN IF NEW.request_id = 'series-130' THEN RAISE EXCEPTION 'injected page failure'; END IF; RETURN NEW; END; $$ LANGUAGE plpgsql; CREATE TRIGGER fail_last_page BEFORE UPDATE ON \"LiteLLM_AutoRouterBaselineObservation\" FOR EACH ROW EXECUTE FUNCTION fail_last_page()";
const LOG_ESTIMATE: &str = r#"SELECT metadata->'autorouter_savings_estimate'->>'reason', metadata->>'keep' FROM "LiteLLM_SpendLogs" WHERE request_id = $1"#;

async fn count(client: &Client, sql: &str) -> i64 {
    client.query_one(sql, &[]).await.unwrap().get(0)
}

// 127 single start times, a tie of two at 128 and one more at 129: the first page holds 129 rows.
async fn seed_series(writer: &PostgresStore, client: &Client) {
    let accounting = store(writer);
    for index in 1_u32..=130_u32 {
        let started_at = match index {
            1..=127 => f64::from(index),
            128 | 129 => 128.0_f64,
            _ => 129.0_f64,
        };
        let value = record(&format!("series-{index}"), started_at);
        assert_eq!(accounting.append(&value).await, AppendResult::Recorded);
    }
    client.batch_execute(SERIES_LOGS).await.unwrap();
}

#[tokio::test]
async fn append_records_the_observation_its_turn_and_a_dirty_comparison() {
    let harness = harness().await;
    let client = harness.pool.get().await.unwrap();
    let accounting = store(&harness.writer);
    assert_eq!(
        accounting.append(&record("req-1", 2.0_f64)).await,
        AppendResult::Recorded
    );
    let state = comparison(&client, SCOPE).await;
    assert_eq!((state.revision, state.published), (1, 0));
    assert!(state.unattempted && !state.retired && state.initial_equivalent);
    let stored = observation(&client, "req-1").await.unwrap();
    assert_eq!((stored.revision, stored.conflicted), (1, false));
    assert!(stored.publication.is_none());
    assert_eq!(session(&client).await.0, 1);
    drop(client);
    harness.close().await;
}

#[tokio::test]
async fn an_identical_retry_neither_rewrites_its_turn_nor_dirties_the_comparison() {
    let harness = harness().await;
    let client = harness.pool.get().await.unwrap();
    let accounting = store(&harness.writer);
    let value = record("req-1", 2.0_f64);
    assert_eq!(accounting.append(&value).await, AppendResult::Recorded);
    client.batch_execute(STAMP_ATTEMPT).await.unwrap();
    assert_eq!(accounting.append(&value).await, AppendResult::Recorded);
    let state = comparison(&client, SCOPE).await;
    assert_eq!(state.revision, 1);
    assert!(!state.unattempted);
    let stored = observation(&client, "req-1").await.unwrap();
    assert_eq!((stored.revision, stored.conflicted), (1, false));
    assert_eq!(session(&client).await.0, 1);
    drop(client);
    harness.close().await;
}

#[tokio::test]
async fn a_new_comparison_is_not_equivalent_once_its_key_session_exists() {
    let harness = harness().await;
    let client = harness.pool.get().await.unwrap();
    let accounting = store(&harness.writer);
    let mut other = record("req-2", 3.0_f64);
    other.scope = OTHER_SCOPE.to_owned();
    assert_eq!(
        accounting.append(&record("req-1", 2.0_f64)).await,
        AppendResult::Recorded
    );
    assert_eq!(accounting.append(&other).await, AppendResult::Recorded);
    assert!(comparison(&client, SCOPE).await.initial_equivalent);
    assert!(!comparison(&client, OTHER_SCOPE).await.initial_equivalent);
    drop(client);
    harness.close().await;
}

#[tokio::test]
async fn a_conflicting_duplicate_is_marked_once_and_withdraws_published_savings() {
    let harness = harness().await;
    let client = harness.pool.get().await.unwrap();
    let accounting = store(&harness.writer);
    seed_logs(&client, &["req-1"]).await;
    assert_eq!(
        accounting.append(&record("req-1", 2.0_f64)).await,
        AppendResult::Recorded
    );
    assert_eq!(accounting.project(SCOPE).await, ProjectResult::Published);
    assert_eq!(session(&client).await, (1, "-0.05".to_owned(), 1));
    let mut differing = record("req-1", 2.0_f64);
    differing.observation.baseline_equivalent = false;
    assert_eq!(accounting.append(&differing).await, AppendResult::Recorded);
    let stored = observation(&client, "req-1").await.unwrap();
    assert_eq!((stored.revision, stored.conflicted), (2, true));
    let pending = stored.publication.unwrap();
    assert_eq!(pending.status, PublicationStatus::Unknown);
    assert_eq!(pending.reason, "pending_projection");
    assert_eq!(pending.comparison_started_at.to_bits(), 2.0_f64.to_bits());
    assert_eq!(session(&client).await, (1, "0".to_owned(), 0));
    let state = comparison(&client, SCOPE).await;
    assert_eq!((state.revision, state.published), (2, 1));
    assert!(state.unattempted);
    client.batch_execute(STAMP_ATTEMPT).await.unwrap();
    differing.observation.reason = Some("third".to_owned());
    assert_eq!(accounting.append(&differing).await, AppendResult::Recorded);
    assert_eq!(observation(&client, "req-1").await.unwrap().revision, 2);
    let unchanged = comparison(&client, SCOPE).await;
    assert_eq!(unchanged.revision, 2);
    assert!(!unchanged.unattempted);
    assert_eq!(accounting.project(SCOPE).await, ProjectResult::Published);
    assert_eq!(reason(&client, "req-1").await, "conflicting_observation");
    drop(client);
    harness.close().await;
}

#[tokio::test]
async fn a_request_id_owned_by_another_scope_is_a_conflict() {
    let harness = harness().await;
    let client = harness.pool.get().await.unwrap();
    let accounting = store(&harness.writer);
    let mut foreign = record("req-1", 2.0_f64);
    foreign.scope = OTHER_SCOPE.to_owned();
    assert_eq!(
        accounting.append(&record("req-1", 2.0_f64)).await,
        AppendResult::Recorded
    );
    assert_eq!(accounting.append(&foreign).await, AppendResult::Conflict);
    assert_eq!(observation(&client, "req-1").await.unwrap().scope, SCOPE);
    assert_eq!(comparison(&client, OTHER_SCOPE).await.revision, 0);
    assert_eq!(session(&client).await.0, 1);
    drop(client);
    harness.close().await;
}

#[tokio::test]
async fn a_retired_comparison_publishes_comparison_retired_and_reverses_the_canonical_estimate() {
    let harness = harness().await;
    let client = harness.pool.get().await.unwrap();
    let accounting = store(&harness.writer);
    seed_logs(&client, &["req-1", "req-2"]).await;
    assert_eq!(
        accounting.append(&record("req-1", 2.0_f64)).await,
        AppendResult::Recorded
    );
    assert_eq!(accounting.project(SCOPE).await, ProjectResult::Published);
    client.batch_execute(RETIRE_SCOPE).await.unwrap();
    let mut differing = record("req-1", 9.0_f64);
    differing.observation.baseline_equivalent = false;
    assert_eq!(accounting.append(&differing).await, AppendResult::Retired);
    let canonical = observation(&client, "req-1")
        .await
        .unwrap()
        .publication
        .unwrap();
    assert_eq!(canonical.reason, "comparison_retired");
    assert_eq!(canonical.comparison_started_at.to_bits(), 2.0_f64.to_bits());
    assert_eq!(session(&client).await, (1, "0".to_owned(), 0));
    assert_eq!(
        accounting.append(&record("req-2", 3.0_f64)).await,
        AppendResult::Retired
    );
    let fresh = observation(&client, "req-2")
        .await
        .unwrap()
        .publication
        .unwrap();
    assert_eq!(fresh.reason, "comparison_retired");
    assert_eq!(fresh.comparison_started_at.to_bits(), 3.0_f64.to_bits());
    let log = client.query_one(LOG_ESTIMATE, &[&"req-2"]).await.unwrap();
    assert_eq!(
        log.get::<_, Option<String>>(0).as_deref(),
        Some("comparison_retired")
    );
    assert_eq!(log.get::<_, Option<String>>(1).as_deref(), Some("value"));
    assert_eq!(session(&client).await.0, 2);
    assert_eq!(comparison(&client, SCOPE).await.revision, 1);
    drop(client);
    harness.close().await;
}

#[tokio::test]
async fn projection_waits_for_the_spend_log_row_and_stays_dirty() {
    let harness = harness().await;
    let client = harness.pool.get().await.unwrap();
    let accounting = store(&harness.writer);
    assert_eq!(
        accounting.append(&record("req-1", 2.0_f64)).await,
        AppendResult::Recorded
    );
    assert_eq!(accounting.project(SCOPE).await, ProjectResult::Unavailable);
    assert_eq!(comparison(&client, SCOPE).await.published, 0);
    assert!(
        observation(&client, "req-1")
            .await
            .unwrap()
            .publication
            .is_none()
    );
    seed_logs(&client, &["req-1"]).await;
    assert_eq!(accounting.project(SCOPE).await, ProjectResult::Published);
    let state = comparison(&client, SCOPE).await;
    assert_eq!(state.published, 1);
    let history: BaselineHistory = serde_json::from_str(&state.history.unwrap()).unwrap();
    assert_eq!(history.last_at, Some(2.0_f64));
    let published = observation(&client, "req-1")
        .await
        .unwrap()
        .publication
        .unwrap();
    assert_eq!(published.status, PublicationStatus::Estimated);
    assert_eq!(published.reason, "identical_baseline_path");
    drop(client);
    harness.close().await;
}

// What the comparison looks like before the projection that must leave it unchanged.
#[derive(Clone, Copy)]
enum Setup {
    Absent,
    Clean,
    Retired,
}

#[rstest]
#[case::absent(Setup::Absent)]
#[case::clean(Setup::Clean)]
#[case::retired(Setup::Retired)]
#[tokio::test]
async fn projection_leaves_absent_clean_and_retired_comparisons_unchanged(#[case] setup: Setup) {
    let harness = harness().await;
    let client = harness.pool.get().await.unwrap();
    let accounting = store(&harness.writer);
    seed_logs(&client, &["req-1"]).await;
    match setup {
        Setup::Absent => {}
        Setup::Clean => {
            assert_eq!(
                accounting.append(&record("req-1", 2.0_f64)).await,
                AppendResult::Recorded
            );
            assert_eq!(accounting.project(SCOPE).await, ProjectResult::Published);
        }
        Setup::Retired => {
            assert_eq!(
                accounting.append(&record("req-1", 2.0_f64)).await,
                AppendResult::Recorded
            );
            client.batch_execute(RETIRE_SCOPE).await.unwrap();
        }
    }
    assert_eq!(accounting.project(SCOPE).await, ProjectResult::Unchanged);
    if matches!(setup, Setup::Retired) {
        assert_eq!(comparison(&client, SCOPE).await.published, 0);
        assert!(
            observation(&client, "req-1")
                .await
                .unwrap()
                .publication
                .is_none()
        );
    }
    drop(client);
    harness.close().await;
}

#[tokio::test]
async fn an_initial_comparison_that_is_not_equivalent_replays_without_the_identical_path() {
    let harness = harness().await;
    let client = harness.pool.get().await.unwrap();
    let accounting = store(&harness.writer);
    let mut other = record("req-2", 3.0_f64);
    other.scope = OTHER_SCOPE.to_owned();
    seed_logs(&client, &["req-1", "req-2"]).await;
    assert_eq!(
        accounting.append(&record("req-1", 2.0_f64)).await,
        AppendResult::Recorded
    );
    assert_eq!(accounting.append(&other).await, AppendResult::Recorded);
    assert_eq!(
        accounting.project(OTHER_SCOPE).await,
        ProjectResult::Published
    );
    assert_eq!(reason(&client, "req-2").await, "unsupported_cache_plan");
    drop(client);
    harness.close().await;
}

#[tokio::test]
async fn a_late_observation_before_the_checkpoint_forces_a_full_replay() {
    let harness = harness().await;
    let client = harness.pool.get().await.unwrap();
    let accounting = store(&harness.writer);
    seed_logs(&client, &["req-1", "req-2", "req-3"]).await;
    assert_eq!(
        accounting.append(&record("req-1", 1.0_f64)).await,
        AppendResult::Recorded
    );
    assert_eq!(
        accounting.append(&record("req-3", 3.0_f64)).await,
        AppendResult::Recorded
    );
    assert_eq!(accounting.project(SCOPE).await, ProjectResult::Published);
    let mut late = record("req-2", 2.0_f64);
    late.observation.baseline_equivalent = false;
    assert_eq!(accounting.append(&late).await, AppendResult::Recorded);
    let withdrawn = observation(&client, "req-3")
        .await
        .unwrap()
        .publication
        .unwrap();
    assert_eq!(withdrawn.reason, "pending_projection");
    assert_eq!(withdrawn.comparison_started_at.to_bits(), 1.0_f64.to_bits());
    assert_eq!(reason(&client, "req-1").await, "identical_baseline_path");
    assert_eq!(comparison(&client, SCOPE).await.revision, 3);
    assert_eq!(accounting.project(SCOPE).await, ProjectResult::Published);
    assert_eq!(reason(&client, "req-1").await, "identical_baseline_path");
    assert_eq!(reason(&client, "req-2").await, "unsupported_cache_plan");
    assert_eq!(reason(&client, "req-3").await, "unsupported_cache_plan");
    drop(client);
    harness.close().await;
}

#[tokio::test]
async fn a_newer_observation_replays_only_later_revisions_from_the_checkpoint() {
    let harness = harness().await;
    let client = harness.pool.get().await.unwrap();
    let accounting = store(&harness.writer);
    seed_logs(&client, &["req-1", "req-5"]).await;
    assert_eq!(
        accounting.append(&record("req-1", 1.0_f64)).await,
        AppendResult::Recorded
    );
    assert_eq!(accounting.project(SCOPE).await, ProjectResult::Published);
    client
        .batch_execute(r#"UPDATE "LiteLLM_AutoRouterBaselineObservation" SET publication = replace(publication, 'identical_baseline_path', 'tampered')"#)
        .await
        .unwrap();
    assert_eq!(
        accounting.append(&record("req-5", 5.0_f64)).await,
        AppendResult::Recorded
    );
    assert_eq!(accounting.project(SCOPE).await, ProjectResult::Published);
    assert_eq!(reason(&client, "req-1").await, "tampered");
    let newer = observation(&client, "req-5")
        .await
        .unwrap()
        .publication
        .unwrap();
    assert_eq!(newer.reason, "identical_baseline_path");
    assert_eq!(newer.comparison_started_at.to_bits(), 1.0_f64.to_bits());
    drop(client);
    harness.close().await;
}

#[tokio::test]
async fn pages_keep_a_tied_start_time_group_together() {
    let harness = harness().await;
    let client = harness.pool.get().await.unwrap();
    seed_series(&harness.writer, &client).await;
    let accounting = store(&harness.writer);
    assert_eq!(accounting.project(SCOPE).await, ProjectResult::Published);
    assert_eq!(count(&client, SERIES_IDENTICAL).await, 130);
    let state = comparison(&client, SCOPE).await;
    let history: BaselineHistory = serde_json::from_str(&state.history.unwrap()).unwrap();
    assert_eq!(history.last_at, Some(129.0_f64));
    assert_eq!(state.published, state.revision);
    drop(client);
    harness.close().await;
}

#[tokio::test]
async fn a_failed_page_rolls_back_every_published_page_and_stays_dirty() {
    let harness = harness().await;
    let client = harness.pool.get().await.unwrap();
    seed_series(&harness.writer, &client).await;
    client.batch_execute(FAIL_LAST_PAGE).await.unwrap();
    let accounting = store(&harness.writer);
    assert_eq!(accounting.project(SCOPE).await, ProjectResult::Unavailable);
    assert_eq!(count(&client, PUBLISHED_ROWS).await, 0);
    assert_eq!(count(&client, ESTIMATED_LOGS).await, 0);
    assert_eq!(session(&client).await.1, "0");
    let state = comparison(&client, SCOPE).await;
    assert_eq!(state.published, 0);
    assert!(state.history.is_none());
    client
        .batch_execute(r#"DROP TRIGGER fail_last_page ON "LiteLLM_AutoRouterBaselineObservation""#)
        .await
        .unwrap();
    assert_eq!(accounting.project(SCOPE).await, ProjectResult::Published);
    assert_eq!(count(&client, PUBLISHED_ROWS).await, 130);
    drop(client);
    harness.close().await;
}

#[tokio::test]
async fn a_held_comparison_lock_fails_the_append_at_the_lock_timeout() {
    let harness = harness().await;
    let client = harness.pool.get().await.unwrap();
    let accounting = store(&harness.writer);
    assert_eq!(
        accounting.append(&record("req-1", 2.0_f64)).await,
        AppendResult::Recorded
    );
    client.batch_execute(LOCK_SCOPE).await.unwrap();
    let started = Instant::now();
    assert_eq!(
        accounting.append(&record("req-2", 3.0_f64)).await,
        AppendResult::Unavailable
    );
    assert!(started.elapsed() < Duration::from_secs(4));
    client.batch_execute("COMMIT").await.unwrap();
    assert!(observation(&client, "req-2").await.is_none());
    assert_eq!(
        accounting.append(&record("req-2", 3.0_f64)).await,
        AppendResult::Recorded
    );
    drop(client);
    harness.close().await;
}

#[tokio::test]
async fn a_store_without_a_writer_reports_both_operations_unavailable() {
    let writer = PostgresStore { pool: None };
    let accounting = store(&writer);
    assert_eq!(
        accounting.append(&record("req-1", 2.0_f64)).await,
        AppendResult::Unavailable
    );
    assert_eq!(accounting.project(SCOPE).await, ProjectResult::Unavailable);
    assert!(matches!(
        accounting.claim_dirty().await,
        Err(StoreError::Writer(_))
    ));
}

#[tokio::test]
async fn retirement_marks_old_comparisons_and_deletes_their_observations_in_batches() {
    let harness = harness().await;
    let mut client = harness.pool.get().await.unwrap();
    let accounting = store(&harness.writer);
    let mut fresh = record("req-3", 3.0_f64);
    fresh.scope = OTHER_SCOPE.to_owned();
    seed_logs(&client, &["req-1", "req-2"]).await;
    assert_eq!(
        accounting.append(&record("req-1", 1.0_f64)).await,
        AppendResult::Recorded
    );
    assert_eq!(
        accounting.append(&record("req-2", 2.0_f64)).await,
        AppendResult::Recorded
    );
    assert_eq!(accounting.append(&fresh).await, AppendResult::Recorded);
    assert_eq!(accounting.project(SCOPE).await, ProjectResult::Published);
    let before = session(&client).await;
    client.batch_execute(AGE_SCOPE).await.unwrap();
    let cutoff = Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap();
    retire_before(&mut client, cutoff, 1, 1000).await.unwrap();
    let retired = comparison(&client, SCOPE).await;
    assert!(retired.retired);
    assert!(retired.history.is_none());
    let kept = [
        observation(&client, "req-1").await.is_some(),
        observation(&client, "req-2").await.is_some(),
    ];
    assert_eq!(kept.iter().filter(|present| **present).count(), 1);
    assert!(!comparison(&client, OTHER_SCOPE).await.retired);
    assert!(observation(&client, "req-3").await.is_some());
    assert_eq!(session(&client).await, before);
    retire_before(&mut client, cutoff, 1, 1000).await.unwrap();
    assert!(observation(&client, "req-1").await.is_none());
    assert!(observation(&client, "req-2").await.is_none());
    drop(client);
    harness.close().await;
}

#[tokio::test]
async fn retirement_skips_a_comparison_another_transaction_holds() {
    let harness = harness().await;
    let holder = harness.pool.get().await.unwrap();
    let mut client = harness.pool.get().await.unwrap();
    let accounting = store(&harness.writer);
    assert_eq!(
        accounting.append(&record("req-1", 1.0_f64)).await,
        AppendResult::Recorded
    );
    client.batch_execute(AGE_SCOPE).await.unwrap();
    holder.batch_execute(LOCK_SCOPE).await.unwrap();
    let cutoff = Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap();
    retire_before(&mut client, cutoff, 10, 1000).await.unwrap();
    holder.batch_execute("COMMIT").await.unwrap();
    assert!(!comparison(&client, SCOPE).await.retired);
    assert!(observation(&client, "req-1").await.is_some());
    drop((holder, client));
    harness.close().await;
}

#[rstest]
#[case::batch_beyond_integer(2_147_483_648, 1000, false)]
#[case::zero_timeout_still_bounds_the_lock_wait(10, 0, true)]
#[tokio::test]
async fn retirement_reports_its_failures(
    #[case] batch_size: i64,
    #[case] timeout_ms: u64,
    #[case] statement: bool,
) {
    let harness = harness().await;
    let holder = harness.pool.get().await.unwrap();
    let mut client = harness.pool.get().await.unwrap();
    holder
        .batch_execute(
            r#"BEGIN; LOCK TABLE "LiteLLM_AutoRouterBaselineComparison" IN ACCESS EXCLUSIVE MODE"#,
        )
        .await
        .unwrap();
    let cutoff = Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap();
    let failure = retire_before(&mut client, cutoff, batch_size, timeout_ms)
        .await
        .unwrap_err();
    assert_eq!(matches!(failure, StoreError::Statement(_)), statement);
    assert_eq!(matches!(failure, StoreError::BatchSize(_)), !statement);
    holder.batch_execute("ROLLBACK").await.unwrap();
    drop((holder, client));
    harness.close().await;
}
