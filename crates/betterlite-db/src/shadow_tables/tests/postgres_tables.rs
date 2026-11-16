#![cfg(test)]

use chrono::{TimeZone as _, Utc};
use deadpool_postgres::Client;
use rust_decimal::Decimal;
use tokio_postgres::Row;

use super::support::database;
use crate::shadow_tables::postgres::PostgresShadowTables;
use crate::shadow_tables::rows::{AttemptOutcome, FunnelCounters, ShadowAttemptRow, ShadowTables};

// one row per gate: live, stopped, expired, and one whose ends_at lands exactly on the read's now
const JOB_ROWS: &str = r#"INSERT INTO "LiteLLM_ShadowEvalJob"
    ("id", "group_id", "target_type", "target_id", "router_name", "router_names", "models",
     "direction", "baseline_model", "shadow_percentage", "judge_model", "max_turns",
     "max_budget", "ends_at", "stopped_at")
VALUES
    ('live', 'live', 'key', 'k-live', 'router-a', ARRAY['router-a', 'router-b'],
     ARRAY['gpt-4o'], 'forward', NULL, 100, 'judge-model', 5, 2.5,
     '2030-01-01 00:00:00', NULL),
    ('stopped', 'stopped', 'key', 'k-stopped', 'router-a', ARRAY[]::TEXT[], ARRAY[]::TEXT[],
     'forward', NULL, 10, 'judge-model', 5, NULL,
     '2030-01-01 00:00:00', '2029-01-01 00:00:00'),
    ('expired', 'expired', 'team', 't-expired', 'router-c', ARRAY[]::TEXT[],
     ARRAY['gpt-4o-mini'], 'reverse', 'gpt-4o', 10, 'judge-model', 5, NULL,
     '2025-01-01 00:00:00', NULL),
    ('boundary', 'boundary', 'user', 'u-boundary', 'router-d', ARRAY[]::TEXT[],
     ARRAY[]::TEXT[], 'forward', NULL, 10, 'judge-model', 5, NULL,
     '2026-11-15 00:00:00', NULL)"#;

const ATTEMPT_ROWS: &str = r#"INSERT INTO "LiteLLM_ShadowEvalAttempt"
    ("id", "job_id", "request_id", "outcome", "judge_cost", "shadow_cost",
     "shadow_classifier_cost", "real_cost", "real_classifier_cost")
VALUES
    ('a1', 'job-a', 'r1', 'shadow', 0.25, 0.5, 0.125, 9, 0),
    ('a2', 'job-a', 'r2', 'real', 0.25, 0, 0, NULL, 0),
    ('b1', 'job-b', 'r3', 'tie', 0, 0, 0, NULL, 0),
    ('z1', 'job-z', 'r4', 'error', 1, 1, 1, NULL, 0)"#;

#[tokio::test]
async fn shadow_eval_job_find_active_filters_stopped_and_expired() {
    let (fixture, pool) = database().await;
    let client = pool.get().await.unwrap();
    client.batch_execute(JOB_ROWS).await.unwrap();
    let tables = PostgresShadowTables { client };
    let now = Utc.with_ymd_and_hms(2026, 11, 15, 0, 0, 0).unwrap();
    let jobs = tables.find_active_jobs(now).await.unwrap();
    fixture.close_async().await.unwrap();
    assert_eq!(jobs.len(), 1);
    let job = jobs.first().unwrap();
    assert_eq!(job.id, "live");
    assert_eq!(job.target_type, "key");
    assert_eq!(job.target_id, "k-live");
    assert_eq!(job.router_name, "router-a");
    assert_eq!(job.router_names, vec!["router-a", "router-b"]);
    assert_eq!(job.models, vec!["gpt-4o"]);
    assert_eq!(job.direction, "forward");
    assert_eq!(job.baseline_model, None);
    assert_eq!(job.judge_model, "judge-model");
    assert_eq!(job.max_turns, 5);
    assert_eq!(job.max_budget, Some(Decimal::new(25, 1)));
    assert_eq!(
        job.ends_at,
        Utc.with_ymd_and_hms(2030, 1, 1, 0, 0, 0).unwrap()
    );
}

#[tokio::test]
async fn shadow_eval_job_find_active_keeps_a_null_budget_null() {
    let (fixture, pool) = database().await;
    let client = pool.get().await.unwrap();
    client
        .batch_execute(
            r#"INSERT INTO "LiteLLM_ShadowEvalJob"
                ("id", "group_id", "target_type", "target_id", "router_name",
                 "shadow_percentage", "judge_model", "max_turns", "ends_at")
               VALUES ('no-budget', 'no-budget', 'team', 't-1', 'router-a',
                       10, 'judge-model', 3, '2030-01-01 00:00:00')"#,
        )
        .await
        .unwrap();
    let tables = PostgresShadowTables { client };
    let now = Utc.with_ymd_and_hms(2026, 11, 15, 0, 0, 0).unwrap();
    let jobs = tables.find_active_jobs(now).await.unwrap();
    fixture.close_async().await.unwrap();
    assert_eq!(jobs.len(), 1);
    let job = jobs.first().unwrap();
    assert_eq!(job.max_budget, None);
    assert!(job.router_names.is_empty());
    assert!(job.models.is_empty());
    assert_eq!(job.direction, "forward");
    assert!(job.baseline_model.is_none());
}

#[tokio::test]
async fn shadow_eval_job_attempt_stats_groups_counts_and_sums() {
    let (fixture, pool) = database().await;
    let client = pool.get().await.unwrap();
    client.batch_execute(ATTEMPT_ROWS).await.unwrap();
    let tables = PostgresShadowTables { client };
    let stats = tables
        .attempt_stats(&["job-a".to_owned(), "job-b".to_owned(), "job-c".to_owned()])
        .await
        .unwrap();
    let empty = tables.attempt_stats(&[]).await.unwrap();
    fixture.close_async().await.unwrap();
    assert_eq!(stats.len(), 2);
    let job_a = stats.iter().find(|row| row.job_id == "job-a").unwrap();
    assert_eq!(job_a.attempts, 2);
    assert_eq!(job_a.spend, Decimal::new(1125, 3));
    let job_b = stats.iter().find(|row| row.job_id == "job-b").unwrap();
    assert_eq!(job_b.attempts, 1);
    assert_eq!(job_b.spend, Decimal::ZERO);
    assert!(empty.is_empty());
}

#[tokio::test]
async fn shadow_eval_attempt_inserts_every_cost() {
    let (fixture, pool) = database().await;
    let client = pool.get().await.unwrap();
    let tables = PostgresShadowTables { client };
    tables.insert_attempt(&verdict_attempt()).await.unwrap();
    tables.insert_attempt(&errored_attempt()).await.unwrap();
    let stored = stored_verdict(&tables.client).await;
    let failed = tables
        .client
        .query_one(
            r#"SELECT "outcome", "router_name", "tier", "real_model", "error",
                      "judge_cost"::numeric AS "judge_cost"
               FROM "LiteLLM_ShadowEvalAttempt" WHERE "request_id" = 'req-error'"#,
            &[],
        )
        .await
        .unwrap();
    fixture.close_async().await.unwrap();
    assert_stored_verdict(&stored);
    assert_eq!(failed.get::<_, String>("outcome"), "error");
    assert_eq!(failed.get::<_, String>("router_name"), "router-b");
    assert_eq!(failed.get::<_, Option<String>>("tier"), None);
    assert_eq!(failed.get::<_, Option<String>>("real_model"), None);
    assert_eq!(
        failed.get::<_, Option<String>>("error"),
        Some("judge replied with no content".to_owned())
    );
    assert_eq!(
        failed.get::<_, Option<Decimal>>("judge_cost"),
        Some(Decimal::ZERO)
    );
}

fn assert_stored_verdict(stored: &Row) {
    assert_eq!(stored.get::<_, String>("job_id"), "job-a");
    assert_eq!(stored.get::<_, String>("router_name"), "router-a");
    assert_eq!(stored.get::<_, String>("outcome"), "shadow");
    assert_eq!(
        stored.get::<_, Option<String>>("tier"),
        Some("tier-shadow".to_owned())
    );
    assert_eq!(
        stored.get::<_, Option<String>>("real_model"),
        Some("gpt-4o".to_owned())
    );
    assert_eq!(
        stored.get::<_, Option<String>>("shadow_model"),
        Some("gpt-4o-mini".to_owned())
    );
    assert_eq!(
        stored.get::<_, Option<Decimal>>("confidence"),
        Some(Decimal::new(875, 3))
    );
    assert_cost(stored, "judge_cost", Some(Decimal::new(25, 2)));
    assert_cost(stored, "shadow_cost", Some(Decimal::new(5, 1)));
    assert_cost(stored, "shadow_classifier_cost", Some(Decimal::new(125, 3)));
    assert_cost(stored, "real_cost", Some(Decimal::new(75, 2)));
    assert_cost(stored, "real_classifier_cost", Some(Decimal::new(625, 4)));
    assert!(stored.get::<_, bool>("real_cache_hit"));
    assert_eq!(stored.get::<_, Option<String>>("error"), None);
}

fn assert_cost(row: &Row, column: &str, expected: Option<Decimal>) {
    assert_eq!(row.get::<_, Option<Decimal>>(column), expected, "{column}");
}

fn verdict_attempt() -> ShadowAttemptRow {
    ShadowAttemptRow {
        job_id: "job-a".to_owned(),
        request_id: "req-verdict".to_owned(),
        router_name: "router-a".to_owned(),
        outcome: AttemptOutcome::Shadow,
        tier: Some("tier-shadow".to_owned()),
        real_model: Some("gpt-4o".to_owned()),
        shadow_model: Some("gpt-4o-mini".to_owned()),
        confidence: Some(Decimal::new(875, 3)),
        judge_cost: Decimal::new(25, 2),
        shadow_cost: Decimal::new(5, 1),
        shadow_classifier_cost: Decimal::new(125, 3),
        real_cost: Some(Decimal::new(75, 2)),
        real_classifier_cost: Decimal::new(625, 4),
        real_cache_hit: true,
        error: None,
    }
}

// the same job's error row: only the error path's fields differ from the verdict row
fn errored_attempt() -> ShadowAttemptRow {
    let mut row = verdict_attempt();
    row.request_id = "req-error".to_owned();
    row.router_name = "router-b".to_owned();
    row.outcome = AttemptOutcome::Error;
    row.tier = None;
    row.real_model = None;
    row.shadow_model = None;
    row.confidence = None;
    row.judge_cost = Decimal::ZERO;
    row.shadow_cost = Decimal::ZERO;
    row.shadow_classifier_cost = Decimal::ZERO;
    row.real_cost = None;
    row.real_classifier_cost = Decimal::ZERO;
    row.real_cache_hit = false;
    row.error = Some("judge replied with no content".to_owned());
    row
}

async fn stored_verdict(client: &Client) -> Row {
    client
        .query_one(
            r#"SELECT "job_id", "router_name", "outcome", "tier", "real_model", "shadow_model",
                      "confidence"::numeric AS "confidence",
                      "judge_cost"::numeric AS "judge_cost",
                      "shadow_cost"::numeric AS "shadow_cost",
                      "shadow_classifier_cost"::numeric AS "shadow_classifier_cost",
                      "real_cost"::numeric AS "real_cost",
                      "real_classifier_cost"::numeric AS "real_classifier_cost",
                      "real_cache_hit", "error"
               FROM "LiteLLM_ShadowEvalAttempt" WHERE "request_id" = 'req-verdict'"#,
            &[],
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn shadow_eval_funnel_upsert_accumulates_each_stage() {
    let (fixture, pool) = database().await;
    let client = pool.get().await.unwrap();
    let tables = PostgresShadowTables { client };
    let first = FunnelCounters {
        not_sampled: 1,
        unjudgeable: 2,
        shed: 0,
        withheld: 0,
    };
    let second = FunnelCounters {
        not_sampled: 0,
        unjudgeable: 1,
        shed: 3,
        withheld: 1,
    };
    tables.upsert_funnel("job-a", first).await.unwrap();
    tables.upsert_funnel("job-a", second).await.unwrap();
    let row = tables
        .client
        .query_one(
            r#"SELECT "not_sampled", "unjudgeable", "shed", "withheld"
               FROM "LiteLLM_ShadowEvalFunnel" WHERE "job_id" = 'job-a'"#,
            &[],
        )
        .await
        .unwrap();
    fixture.close_async().await.unwrap();
    assert_eq!(row.get::<_, i32>("not_sampled"), 1);
    assert_eq!(row.get::<_, i32>("unjudgeable"), 3);
    assert_eq!(row.get::<_, i32>("shed"), 3);
    assert_eq!(row.get::<_, i32>("withheld"), 1);
}
