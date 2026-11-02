#![cfg(test)]

use super::{FinishClaim, SyncStore, iso_format};
use betterlite_postgres_fixture::connection::PostgresFixture;
use betterlite_roi::records::{
    ROIEstimate, ROIEstimateStatus, ROIPullRecord, ROIReport, ROISyncPhase, ROISyncStatus,
};
use chrono::NaiveDate;
use clap::Parser;
use deadpool_postgres::{Client, Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn database() -> (PostgresFixture, Pool, Client) {
    let url = Environment::parse_from(["sync-store-tests"])
        .url
        .map(SecretString::from);
    let fixture = PostgresFixture::provision_schema(url.as_ref())
        .resolve()
        .await
        .unwrap();
    let manager = Manager::new(fixture.options.url.expose_secret().parse().unwrap(), NoTls);
    let pool = Pool::builder(manager).max_size(1).build().unwrap();
    let client = pool.get().await.unwrap();
    client
        .batch_execute(include_str!("schema.sql"))
        .await
        .unwrap();
    (fixture, pool, client)
}

fn sample_status(running: bool) -> ROISyncStatus {
    ROISyncStatus {
        running,
        phase: ROISyncPhase::Spend,
        stage: "Reading spend".to_owned(),
        done: 1,
        total: 4,
        estimated: 0,
        reused: 0,
        needs_attention: 0,
        error: None,
        started_at: Some("2026-01-01T00:00:00+00:00".to_owned()),
        finished_at: None,
        next_update: None,
        elapsed_seconds: 5,
        remaining_seconds: Some(10),
    }
}

fn sample_pull(url: &str, cache_key: Option<&str>) -> ROIPullRecord {
    ROIPullRecord {
        repo: "acme/widgets".to_owned(),
        number: 1,
        title: "Add widget".to_owned(),
        url: url.to_owned(),
        login: "octocat".to_owned(),
        emails: vec!["octocat@example.com".to_owned()],
        profile_email: "octocat@example.com".to_owned(),
        commit_emails: None,
        merged_at: "2026-01-01T00:00:00Z".to_owned(),
        head_sha: "abc123".to_owned(),
        additions: 10,
        deletions: 2,
        changed_files: 1,
        commit_count: 1,
        incomplete_metadata: false,
        estimate: ROIEstimate {
            status: ROIEstimateStatus::Estimated,
            hours: Some(2.0),
            reasoning: "Small change".to_owned(),
            model: None,
            evidence_source: None,
            effort_basis: None,
            cached: None,
        },
        cache_key: cache_key.map(str::to_owned),
    }
}

fn sample_report(pulls: Vec<ROIPullRecord>) -> ROIReport {
    ROIReport {
        mode: "scheduled".to_owned(),
        start: "2026-01-01".to_owned(),
        end: "2026-01-31".to_owned(),
        synced_at: "2026-02-01T00:00:00+00:00".to_owned(),
        repos: vec!["acme/widgets".to_owned()],
        estimator_model: "gpt-5".to_owned(),
        estimator_prompt: "Estimate the effort.".to_owned(),
        effort_basis: "lines".to_owned(),
        spend: Vec::new(),
        pulls,
        settings_fingerprint: "fingerprint".to_owned(),
        warnings: None,
        unavailable_repos: None,
        id: None,
    }
}

#[derive(serde::Serialize)]
struct StoredSyncState<'state> {
    owner: &'state str,
    status: &'state ROISyncStatus,
    cancel: bool,
}

async fn insert_sync_state(
    client: &Client,
    owner: &str,
    status: &ROISyncStatus,
    cancel: bool,
    age_seconds: f64,
) {
    let value = serde_json::to_string(&StoredSyncState {
        owner,
        status,
        cancel,
    })
    .unwrap();
    client
        .execute(
            r#"INSERT INTO "LiteLLM_Config" (param_name, param_value, last_run_at)
               VALUES ('roi_calculator_sync', $1::text::jsonb, NOW() - ($2::double precision * INTERVAL '1 second'))"#,
            &[&value, &age_seconds],
        )
        .await
        .unwrap();
}

async fn config_param(client: &Client, param_name: &str) -> Option<String> {
    client
        .query_opt(
            r#"SELECT param_value::text FROM "LiteLLM_Config" WHERE param_name = $1"#,
            &[&param_name],
        )
        .await
        .unwrap()
        .map(|row| row.get(0))
}

async fn seed_cached_pulls(client: &Client) {
    client
        .batch_execute(
            r#"
        INSERT INTO "LiteLLM_Config" (param_name, param_value) VALUES
            ('roi_calculator_pull_old-key', '{"url":"https://example.com/pr/1"}'),
            ('roi_calculator_pull_key-1', '{"url":"https://example.com/pr/1"}'),
            ('roi_calculator_pull_unrelated', '{"url":"https://example.com/pr/2"}');
    "#,
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn acquire_succeeds_when_no_existing_lease() {
    let (fixture, pool, client) = database().await;
    let acquired = SyncStore::acquire(&**client, "owner-a", &sample_status(true), 0.0)
        .await
        .unwrap();
    assert!(acquired);
    assert!(config_param(&client, "roi_calculator_sync").await.is_some());
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn acquire_fails_when_lease_is_held_and_running() {
    let (fixture, pool, client) = database().await;
    insert_sync_state(&client, "owner-a", &sample_status(true), false, 0.0).await;
    let acquired = SyncStore::acquire(&**client, "owner-b", &sample_status(true), 0.0)
        .await
        .unwrap();
    assert!(!acquired);
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn acquire_succeeds_when_lease_expired() {
    let (fixture, pool, client) = database().await;
    insert_sync_state(&client, "owner-a", &sample_status(true), false, 120.0).await;
    let acquired = SyncStore::acquire(&**client, "owner-b", &sample_status(true), 0.0)
        .await
        .unwrap();
    assert!(acquired);
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn acquire_succeeds_when_existing_run_finished() {
    let (fixture, pool, client) = database().await;
    insert_sync_state(&client, "owner-a", &sample_status(false), false, 0.0).await;
    let acquired = SyncStore::acquire(&**client, "owner-b", &sample_status(true), 0.0)
        .await
        .unwrap();
    assert!(acquired);
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn acquire_respects_scheduled_interval_window() {
    let (fixture, pool, client) = database().await;
    insert_sync_state(&client, "owner-a", &sample_status(false), false, 30.0).await;
    let blocked = SyncStore::acquire(&**client, "owner-b", &sample_status(true), 10.0)
        .await
        .unwrap();
    assert!(!blocked);
    let allowed = SyncStore::acquire(&**client, "owner-b", &sample_status(true), 0.4)
        .await
        .unwrap();
    assert!(allowed);
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn heartbeat_extends_lease_and_updates_status() {
    let (fixture, pool, client) = database().await;
    insert_sync_state(&client, "owner-a", &sample_status(true), false, 30.0).await;
    let mut status = sample_status(true);
    status.done = 2;
    let extended = SyncStore::heartbeat(&**client, "owner-a", &status)
        .await
        .unwrap();
    assert!(extended);
    let stored = SyncStore::status(&**client).await.unwrap().unwrap();
    assert_eq!(stored.done, 2);
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn heartbeat_fails_on_owner_mismatch() {
    let (fixture, pool, client) = database().await;
    insert_sync_state(&client, "owner-a", &sample_status(true), false, 0.0).await;
    let extended = SyncStore::heartbeat(&**client, "owner-b", &sample_status(true))
        .await
        .unwrap();
    assert!(!extended);
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn heartbeat_fails_when_cancelled() {
    let (fixture, pool, client) = database().await;
    insert_sync_state(&client, "owner-a", &sample_status(true), true, 0.0).await;
    let extended = SyncStore::heartbeat(&**client, "owner-a", &sample_status(true))
        .await
        .unwrap();
    assert!(!extended);
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn heartbeat_fails_when_not_running() {
    let (fixture, pool, client) = database().await;
    insert_sync_state(&client, "owner-a", &sample_status(false), false, 0.0).await;
    let extended = SyncStore::heartbeat(&**client, "owner-a", &sample_status(true))
        .await
        .unwrap();
    assert!(!extended);
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn heartbeat_fails_when_lease_expired() {
    let (fixture, pool, client) = database().await;
    insert_sync_state(&client, "owner-a", &sample_status(true), false, 120.0).await;
    let extended = SyncStore::heartbeat(&**client, "owner-a", &sample_status(true))
        .await
        .unwrap();
    assert!(!extended);
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn finish_requires_ownership() {
    let (fixture, pool, client) = database().await;
    insert_sync_state(&client, "owner-a", &sample_status(true), false, 0.0).await;
    let finished = SyncStore::finish(
        &**client,
        FinishClaim {
            owner: "owner-b",
            status: &sample_status(false),
            report: None,
        },
    )
    .await
    .unwrap();
    assert!(!finished);
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn finish_fails_when_lease_expired() {
    let (fixture, pool, client) = database().await;
    insert_sync_state(&client, "owner-a", &sample_status(true), false, 120.0).await;
    let finished = SyncStore::finish(
        &**client,
        FinishClaim {
            owner: "owner-a",
            status: &sample_status(false),
            report: None,
        },
    )
    .await
    .unwrap();
    assert!(!finished);
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn finish_skips_cancel_check_when_report_is_null() {
    let (fixture, pool, client) = database().await;
    insert_sync_state(&client, "owner-a", &sample_status(true), true, 0.0).await;
    let finished = SyncStore::finish(
        &**client,
        FinishClaim {
            owner: "owner-a",
            status: &sample_status(false),
            report: None,
        },
    )
    .await
    .unwrap();
    assert!(finished);
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn finish_checks_cancel_when_report_is_present() {
    let (fixture, pool, client) = database().await;
    insert_sync_state(&client, "owner-a", &sample_status(true), true, 0.0).await;
    let report = sample_report(Vec::new());
    let finished = SyncStore::finish(
        &**client,
        FinishClaim {
            owner: "owner-a",
            status: &sample_status(false),
            report: Some(&report),
        },
    )
    .await
    .unwrap();
    assert!(!finished);
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn finish_publishes_report_atomically() {
    let (fixture, pool, client) = database().await;
    insert_sync_state(&client, "owner-a", &sample_status(true), false, 0.0).await;
    let report = sample_report(vec![sample_pull("https://example.com/pr/1", Some("key-1"))]);
    let finished = SyncStore::finish(
        &**client,
        FinishClaim {
            owner: "owner-a",
            status: &sample_status(false),
            report: Some(&report),
        },
    )
    .await
    .unwrap();
    assert!(finished);
    let stored = config_param(&client, "roi_calculator_report")
        .await
        .unwrap();
    assert!(stored.contains("https://example.com/pr/1"));
    let status = SyncStore::status(&**client).await.unwrap().unwrap();
    assert!(!status.running);
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn finish_cleans_up_obsolete_cached_pulls() {
    let (fixture, pool, client) = database().await;
    insert_sync_state(&client, "owner-a", &sample_status(true), false, 0.0).await;
    seed_cached_pulls(&client).await;
    let report = sample_report(vec![sample_pull("https://example.com/pr/1", Some("key-1"))]);
    let finished = SyncStore::finish(
        &**client,
        FinishClaim {
            owner: "owner-a",
            status: &sample_status(false),
            report: Some(&report),
        },
    )
    .await
    .unwrap();
    assert!(finished);
    assert!(
        config_param(&client, "roi_calculator_pull_old-key")
            .await
            .is_none()
    );
    assert!(
        config_param(&client, "roi_calculator_pull_key-1")
            .await
            .is_some()
    );
    assert!(
        config_param(&client, "roi_calculator_pull_unrelated")
            .await
            .is_some()
    );
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn status_returns_none_when_no_sync_exists() {
    let (fixture, pool, client) = database().await;
    let status = SyncStore::status(&**client).await.unwrap();
    assert!(status.is_none());
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn status_returns_status_unchanged_when_not_expired() {
    let (fixture, pool, client) = database().await;
    insert_sync_state(&client, "owner-a", &sample_status(true), false, 5.0).await;
    let status = SyncStore::status(&**client).await.unwrap().unwrap();
    assert!(status.running);
    assert_eq!(status.phase, ROISyncPhase::Spend);
    assert!(status.error.is_none());
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn status_marks_expired_worker_as_error() {
    let (fixture, pool, client) = database().await;
    insert_sync_state(&client, "owner-a", &sample_status(true), false, 120.0).await;
    let status = SyncStore::status(&**client).await.unwrap().unwrap();
    assert!(!status.running);
    assert_eq!(status.phase, ROISyncPhase::Error);
    assert_eq!(status.stage, "Sync interrupted");
    assert_eq!(
        status.error,
        Some(
            "The worker stopped responding. Run analysis again to resume saved estimates."
                .to_owned()
        )
    );
    assert!(status.finished_at.is_some());
    assert_eq!(status.done, 1);
    assert_eq!(status.total, 4);
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn status_does_not_rewrite_already_stopped_worker() {
    let (fixture, pool, client) = database().await;
    insert_sync_state(&client, "owner-a", &sample_status(false), false, 120.0).await;
    let status = SyncStore::status(&**client).await.unwrap().unwrap();
    assert!(!status.running);
    assert_eq!(status.phase, ROISyncPhase::Spend);
    assert!(status.error.is_none());
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn cancel_sets_cancel_flag_and_phase_regardless_of_expiry_and_ownership() {
    let (fixture, pool, client) = database().await;
    insert_sync_state(&client, "owner-a", &sample_status(true), false, 120.0).await;
    SyncStore::cancel(&**client).await.unwrap();
    let stored = config_param(&client, "roi_calculator_sync").await.unwrap();
    assert!(stored.contains("\"cancel\":true") || stored.contains("\"cancel\": true"));
    assert!(
        stored.contains("\"phase\":\"cancelled\"") || stored.contains("\"phase\": \"cancelled\"")
    );
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn cancel_is_noop_when_not_running() {
    let (fixture, pool, client) = database().await;
    insert_sync_state(&client, "owner-a", &sample_status(false), false, 0.0).await;
    SyncStore::cancel(&**client).await.unwrap();
    let stored = config_param(&client, "roi_calculator_sync").await.unwrap();
    assert!(stored.contains("\"cancel\":false") || stored.contains("\"cancel\": false"));
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn clear_report_deletes_cached_report() {
    let (fixture, pool, client) = database().await;
    client
        .execute(
            r#"INSERT INTO "LiteLLM_Config" (param_name, param_value) VALUES ('roi_calculator_report', '{}')"#,
            &[],
        )
        .await
        .unwrap();
    SyncStore::clear_report(&**client).await.unwrap();
    assert!(
        config_param(&client, "roi_calculator_report")
            .await
            .is_none()
    );
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn clear_report_is_noop_when_no_report_exists() {
    let (fixture, pool, client) = database().await;
    SyncStore::clear_report(&**client).await.unwrap();
    assert!(
        config_param(&client, "roi_calculator_report")
            .await
            .is_none()
    );
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[test]
fn iso_format_omits_fractional_seconds_when_instant_is_exact() {
    let instant = NaiveDate::from_ymd_opt(2026, 1, 2)
        .unwrap()
        .and_hms_opt(3, 4, 5)
        .unwrap();
    assert_eq!(iso_format(instant), "2026-01-02T03:04:05+00:00");
}

#[test]
fn iso_format_keeps_microseconds_when_instant_has_a_fractional_part() {
    let instant = NaiveDate::from_ymd_opt(2026, 1, 2)
        .unwrap()
        .and_hms_micro_opt(3, 4, 5, 123_456)
        .unwrap();
    assert_eq!(iso_format(instant), "2026-01-02T03:04:05.123456+00:00");
}
