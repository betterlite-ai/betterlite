#![cfg(test)]

use chrono::{DateTime, Utc};
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;

use super::super::aggregation::{AttemptAggRow, GroupBy};
use super::super::error::ShadowResultsError;
use super::super::labels::{LabelKind, LabelRow};
use super::super::legs::LegRow;
use super::super::postgres::{PgShadowResults, leg_row};
use super::super::repository::ShadowResultsRepository as _;
use super::super::responses::{LegCounts, group_response};
use super::super::results::{FunnelTotals, TargetVerdicts};
use super::super::targets::TargetKey;
use super::{agg, assert_unrepresentable, money};
use crate::shadow_eval_job_loading::jobs::ShadowEvalDirection;
use crate::shadow_eval_models::ShadowEvalTargetType;
use crate::shadow_eval_models::ShadowEvalTargetType::{Key as KeyTarget, Team as TeamTarget};

const SCHEMA: &str = r#"
CREATE TABLE "LiteLLM_ShadowEvalJob" (
    "id" TEXT PRIMARY KEY,
    "group_id" TEXT NOT NULL,
    "target_type" TEXT NOT NULL,
    "target_id" TEXT NOT NULL,
    "router_name" TEXT NOT NULL,
    "router_names" TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[],
    "models" TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[],
    "direction" TEXT NOT NULL DEFAULT 'forward',
    "baseline_model" TEXT,
    "shadow_percentage" DOUBLE PRECISION NOT NULL,
    "judge_model" TEXT NOT NULL,
    "max_turns" INTEGER NOT NULL,
    "max_budget" DOUBLE PRECISION,
    "created_at" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "ends_at" TIMESTAMP(3) NOT NULL,
    "stopped_at" TIMESTAMP(3),
    "stopped_by" TEXT
);
CREATE TABLE "LiteLLM_ShadowEvalAttempt" (
    "id" TEXT PRIMARY KEY,
    "job_id" TEXT NOT NULL,
    "request_id" TEXT NOT NULL,
    "router_name" TEXT,
    "outcome" TEXT NOT NULL,
    "tier" TEXT,
    "real_model" TEXT,
    "shadow_model" TEXT,
    "confidence" DOUBLE PRECISION,
    "judge_cost" DOUBLE PRECISION NOT NULL DEFAULT 0,
    "shadow_cost" DOUBLE PRECISION NOT NULL DEFAULT 0,
    "shadow_classifier_cost" DOUBLE PRECISION NOT NULL DEFAULT 0,
    "real_cost" DOUBLE PRECISION,
    "real_classifier_cost" DOUBLE PRECISION NOT NULL DEFAULT 0,
    "real_cache_hit" BOOLEAN NOT NULL DEFAULT FALSE,
    "error" TEXT,
    "created_at" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE "LiteLLM_ShadowEvalFunnel" (
    "job_id" TEXT PRIMARY KEY,
    "not_sampled" INTEGER NOT NULL DEFAULT 0,
    "unjudgeable" INTEGER NOT NULL DEFAULT 0,
    "shed" INTEGER NOT NULL DEFAULT 0,
    "withheld" INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE "LiteLLM_VerificationToken" (
    "token" TEXT PRIMARY KEY,
    "key_alias" TEXT,
    "key_name" TEXT
);
CREATE TABLE "LiteLLM_TeamTable" (
    "team_id" TEXT PRIMARY KEY,
    "team_alias" TEXT
);
CREATE TABLE "LiteLLM_UserTable" (
    "user_id" TEXT PRIMARY KEY,
    "user_email" TEXT
);
"#;

const JOBS: &str = r#"INSERT INTO "LiteLLM_ShadowEvalJob"
    ("id", "group_id", "target_type", "target_id", "router_name", "shadow_percentage",
     "judge_model", "max_turns", "ends_at")
VALUES
    ('leg-a', 'job-1', 'key', 'k1', 'router-job', 10, 'judge', 10000, '2030-01-01 00:00:00'),
    ('leg-b', 'job-1', 'team', 't1', 'router-job', 10, 'judge', 10000, '2030-01-01 00:00:00'),
    ('leg-x', 'job-x', 'user', 'u1', 'router-other', 10, 'judge', 10000, '2030-01-01 00:00:00')"#;

const ATTEMPTS: &str = r#"INSERT INTO "LiteLLM_ShadowEvalAttempt"
    ("id", "job_id", "request_id", "router_name", "outcome", "tier", "real_model",
     "confidence", "judge_cost", "shadow_cost", "shadow_classifier_cost", "real_cost",
     "real_classifier_cost", "real_cache_hit", "error", "created_at")
VALUES
    ('a1', 'leg-a', 'r1', 'router-job', 'real', 'simple', 'gpt-5.2', 0.75, 0.001, 0.5, 0.125, 0.25, 0.125, false, NULL, '2026-01-01 00:00:01'),
    ('a2', 'leg-a', 'r2', 'router-job', 'shadow', NULL, NULL, NULL, 0, 0, 0, 0.5, 0, true, NULL, '2026-01-01 00:00:02'),
    ('a3', 'leg-b', 'r3', 'router-job', 'tie', 'simple', 'gpt-5.2', 0.5, 0, 0, 0, NULL, 0, false, NULL, '2026-01-01 00:00:03'),
    ('a4', 'leg-a', 'r4', 'router-job', 'error', 'simple', NULL, NULL, 0.125, 0, 0, NULL, 0, false, 'boom', '2026-01-01 00:00:04'),
    ('x1', 'leg-x', 'r5', 'router-other', 'real', 'simple', 'gpt-5.2', 0.5, 0, 0, 0, 1.0, 0, false, NULL, '2026-01-01 00:00:05')"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

fn repository(pool: &Pool) -> PgShadowResults {
    PgShadowResults {
        store: PostgresStore {
            pool: Some(pool.clone()),
        },
    }
}

async fn database() -> (PostgresFixture, Pool) {
    let url = Environment::parse_from(["shadow-result-aggregation-tests"])
        .url
        .map(SecretString::from);
    let fixture = PostgresFixture::provision_with_statements(url.as_ref(), vec![SCHEMA.to_owned()])
        .resolve()
        .await
        .unwrap();
    let manager = Manager::new(fixture.options.url.expose_secret().parse().unwrap(), NoTls);
    let pool = Pool::builder(manager).max_size(1).build().unwrap();
    (fixture, pool)
}

async fn execute(pool: &Pool, sql: &str) {
    let client = pool.get().await.unwrap();
    client.batch_execute(sql).await.unwrap();
}

async fn legs(pool: &Pool, ids: &[&str]) -> Vec<LegRow> {
    let owned: Vec<String> = ids.iter().map(|id| (*id).to_owned()).collect();
    let client = pool.get().await.unwrap();
    let rows = client
        .query(
            r#"SELECT * FROM "LiteLLM_ShadowEvalJob" WHERE "id" = ANY($1::text[])"#,
            &[&owned],
        )
        .await
        .unwrap();
    let mut legs: Vec<LegRow> = rows.iter().map(|row| leg_row(row).unwrap()).collect();
    legs.sort_by(|left, right| left.id.cmp(&right.id));
    legs
}

fn row<'rows>(rows: &'rows [AttemptAggRow], group: &str) -> &'rows AttemptAggRow {
    rows.iter().find(|row| row.group == group).unwrap()
}

fn verdict_group(
    verdicts: &TargetVerdicts,
    target_type: ShadowEvalTargetType,
    target_id: &str,
) -> String {
    verdicts
        .get(&TargetKey {
            target_type,
            target_id: target_id.to_owned(),
        })
        .unwrap()
        .group
        .clone()
}

fn simple_tier() -> AttemptAggRow {
    agg("simple", 2, (1, 0, 1, 0), Some(0.625), (0.375, 0.625))
}

fn unclassified_tier() -> AttemptAggRow {
    agg("UNCLASSIFIED", 1, (0, 1, 0, 1), None, (0.0, 0.0))
}

#[tokio::test]
async fn tier_slices_skip_errors_and_cache_served_spend() {
    let (fixture, pool) = database().await;
    execute(&pool, JOBS).await;
    execute(&pool, ATTEMPTS).await;
    let rows = repository(&pool)
        .attempt_aggregates(&["leg-a".to_owned(), "leg-b".to_owned()], GroupBy::Tier)
        .await
        .unwrap();
    pool.close();
    fixture.close_async().await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(row(&rows, "simple"), &simple_tier());
    assert_eq!(row(&rows, "UNCLASSIFIED"), &unclassified_tier());
}

#[tokio::test]
async fn model_slices_name_a_missing_model_unknown() {
    let (fixture, pool) = database().await;
    execute(&pool, JOBS).await;
    execute(&pool, ATTEMPTS).await;
    let rows = repository(&pool)
        .attempt_aggregates(&["leg-a".to_owned(), "leg-b".to_owned()], GroupBy::Model)
        .await
        .unwrap();
    pool.close();
    fixture.close_async().await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(row(&rows, "gpt-5.2").turn_count, 2);
    assert_eq!(row(&rows, "gpt-5.2").avg_confidence, Some(0.625_f64));
    assert_eq!(row(&rows, "unknown").turn_count, 1);
    assert_eq!(row(&rows, "unknown").cache_hit_turns, 1);
}

#[tokio::test]
async fn leg_slices_count_only_the_requested_legs() {
    let (fixture, pool) = database().await;
    execute(&pool, JOBS).await;
    execute(&pool, ATTEMPTS).await;
    let rows = repository(&pool)
        .attempt_aggregates(&["leg-a".to_owned(), "leg-b".to_owned()], GroupBy::Leg)
        .await
        .unwrap();
    pool.close();
    fixture.close_async().await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(row(&rows, "leg-a").turn_count, 2);
    assert_eq!(row(&rows, "leg-a").real_spend, money(0.375));
    assert_eq!(row(&rows, "leg-a").avg_confidence, Some(0.75_f64));
    assert_eq!(row(&rows, "leg-b").turn_count, 1);
}

#[tokio::test]
async fn a_non_finite_spend_names_the_column_it_cannot_represent() {
    let (fixture, pool) = database().await;
    execute(&pool, JOBS).await;
    execute(&pool, ATTEMPTS).await;
    execute(
        &pool,
        r#"UPDATE "LiteLLM_ShadowEvalAttempt" SET "real_cost" = 'Infinity' WHERE "id" = 'a1'"#,
    )
    .await;
    let aggregates = repository(&pool)
        .attempt_aggregates(&["leg-a".to_owned()], GroupBy::Tier)
        .await;
    pool.close();
    fixture.close_async().await.unwrap();
    assert_unrepresentable(aggregates, "real_spend");
}

#[tokio::test]
async fn router_slices_join_the_job_for_an_unstamped_row() {
    let (fixture, pool) = database().await;
    execute(&pool, JOBS).await;
    execute(
        &pool,
        r#"INSERT INTO "LiteLLM_ShadowEvalAttempt"
            ("id", "job_id", "request_id", "router_name", "outcome", "tier", "real_cost")
           VALUES
            ('arm', 'leg-a', 'r1', 'router-arm', 'real', 'simple', 0),
            ('unstamped', 'leg-b', 'r2', NULL, 'tie', 'simple', NULL),
            ('stamped', 'leg-a', 'r3', 'router-job', 'shadow', 'simple', NULL)"#,
    )
    .await;
    let rows = repository(&pool)
        .attempt_aggregates(&["leg-a".to_owned(), "leg-b".to_owned()], GroupBy::Router)
        .await
        .unwrap();
    pool.close();
    fixture.close_async().await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(row(&rows, "router-arm").turn_count, 1);
    assert_eq!(row(&rows, "router-job").turn_count, 2);
}

#[tokio::test]
async fn totals_count_judged_and_errored_attempts_with_judge_spend() {
    let (fixture, pool) = database().await;
    execute(&pool, JOBS).await;
    execute(
        &pool,
        r#"INSERT INTO "LiteLLM_ShadowEvalAttempt"
            ("id", "job_id", "request_id", "outcome", "judge_cost")
           VALUES
            ('j1', 'leg-a', 'r1', 'real', 0.25),
            ('j2', 'leg-a', 'r2', 'tie', 0.5),
            ('e1', 'leg-b', 'r3', 'error', 0.125)"#,
    )
    .await;
    let repo = repository(&pool);
    let ids = vec!["leg-a".to_owned(), "leg-b".to_owned()];
    let totals = repo.attempt_totals(&ids).await.unwrap();
    let empty = repo.attempt_totals(&[]).await.unwrap();
    pool.close();
    fixture.close_async().await.unwrap();
    assert_eq!(totals.judged_count, 2);
    assert_eq!(totals.error_count, 1);
    assert_eq!(totals.judge_spend, money(0.875));
    assert_eq!(empty.judged_count, 0);
    assert_eq!(empty.error_count, 0);
    assert_eq!(empty.judge_spend, money(0.0));
}

#[tokio::test]
async fn attempt_counts_freeze_each_leg_at_its_stopped_at() {
    let (fixture, pool) = database().await;
    execute(
        &pool,
        r#"INSERT INTO "LiteLLM_ShadowEvalJob"
            ("id", "group_id", "target_type", "target_id", "router_name", "shadow_percentage",
             "judge_model", "max_turns", "ends_at", "stopped_at")
           VALUES
            ('leg-a', 'job-1', 'key', 'k1', 'router-job', 10, 'judge', 10000, '2030-01-01 00:00:00', '2026-01-01 00:00:00'),
            ('leg-b', 'job-1', 'team', 't1', 'router-job', 10, 'judge', 10000, '2030-01-01 00:00:00', NULL)"#,
    )
    .await;
    execute(
        &pool,
        r#"INSERT INTO "LiteLLM_ShadowEvalAttempt"
            ("id", "job_id", "request_id", "outcome", "judge_cost", "shadow_cost",
             "shadow_classifier_cost", "created_at")
           VALUES
            ('a1', 'leg-a', 'r1', 'real', 0.25, 0.5, 0.125, '2025-12-31 00:00:00'),
            ('a2', 'leg-a', 'r2', 'real', 1, 1, 1, '2026-01-02 00:00:00'),
            ('b1', 'leg-b', 'r3', 'real', 0.5, 0, 0, '2026-01-02 00:00:00'),
            ('b2', 'leg-b', 'r4', 'real', 0.25, 0, 0, '2026-01-03 00:00:00')"#,
    )
    .await;
    let repo = repository(&pool);
    let ids = vec!["leg-a".to_owned(), "leg-b".to_owned()];
    let rows = repo.attempt_count_rows(&ids).await.unwrap();
    let counted = legs(&pool, &["leg-a", "leg-b"]).await;
    let counts = repo.leg_attempt_counts(&counted).await.unwrap();
    let empty = repo.leg_attempt_counts(&[]).await.unwrap();
    pool.close();
    fixture.close_async().await.unwrap();
    let leg_a = rows.iter().find(|row| row.job_id == "leg-a").unwrap();
    let leg_b = rows.iter().find(|row| row.job_id == "leg-b").unwrap();
    assert_eq!(leg_a.attempt_count, 1);
    assert_eq!(leg_a.spend, money(0.875));
    assert_eq!(leg_b.attempt_count, 2);
    assert_eq!(leg_b.spend, money(0.75));
    assert_eq!(counts.len(), 2);
    assert_eq!(counts["leg-a"].attempt_count, 1);
    assert!(empty.is_empty());
}

#[tokio::test]
async fn funnel_totals_sum_every_stage_and_report_their_row_count() {
    let (fixture, pool) = database().await;
    execute(&pool, JOBS).await;
    execute(
        &pool,
        r#"INSERT INTO "LiteLLM_ShadowEvalFunnel"
            ("job_id", "not_sampled", "unjudgeable", "shed", "withheld")
           VALUES
            ('leg-a', 1, 2, 3, 4),
            ('leg-b', 10, 20, 30, 40),
            ('leg-x', 100, 200, 300, 400)"#,
    )
    .await;
    let repo = repository(&pool);
    let both = repo
        .funnel_totals(&["leg-a".to_owned(), "leg-b".to_owned()])
        .await
        .unwrap();
    let none = repo.funnel_totals(&[]).await.unwrap();
    pool.close();
    fixture.close_async().await.unwrap();
    assert_eq!(
        both,
        Some(FunnelTotals {
            legs_with_rows: 2,
            not_sampled: 11,
            unjudgeable: 22,
            shed: 33,
            withheld: 44,
        })
    );
    assert_eq!(
        none,
        Some(FunnelTotals {
            legs_with_rows: 0,
            not_sampled: 0,
            unjudgeable: 0,
            shed: 0,
            withheld: 0,
        })
    );
}

#[tokio::test]
async fn latest_error_takes_the_newest_error_across_legs() {
    let (fixture, pool) = database().await;
    execute(&pool, JOBS).await;
    execute(
        &pool,
        r#"INSERT INTO "LiteLLM_ShadowEvalAttempt"
            ("id", "job_id", "request_id", "outcome", "error", "created_at")
           VALUES
            ('e1', 'leg-a', 'r1', 'error', 'old', '2026-01-01 00:00:01'),
            ('e2', 'leg-a', 'r2', 'error', 'newer', '2026-01-01 00:00:02'),
            ('e3', 'leg-b', 'r3', 'error', 'newest', '2026-01-01 00:00:03'),
            ('e4', 'leg-b', 'r4', 'real', NULL, '2026-01-01 00:00:04')"#,
    )
    .await;
    let repo = repository(&pool);
    let ids = vec!["leg-a".to_owned(), "leg-b".to_owned()];
    let newest = repo.latest_error(&ids).await.unwrap();
    execute(
        &pool,
        r#"INSERT INTO "LiteLLM_ShadowEvalAttempt"
            ("id", "job_id", "request_id", "outcome", "error", "created_at")
           VALUES ('e5', 'leg-b', 'r5', 'error', NULL, '2026-01-01 00:00:05')"#,
    )
    .await;
    let silent = repo.latest_error(&ids).await.unwrap();
    let absent = repo.latest_error(&["leg-x".to_owned()]).await.unwrap();
    pool.close();
    fixture.close_async().await.unwrap();
    assert_eq!(newest.as_deref(), Some("newest"));
    assert_eq!(silent, None);
    assert_eq!(absent, None);
}

#[tokio::test]
async fn labels_read_each_kind_and_ignore_unknown_ids() {
    let (fixture, pool) = database().await;
    execute(
        &pool,
        r#"INSERT INTO "LiteLLM_VerificationToken" ("token", "key_alias", "key_name")
               VALUES ('k1', 'key alias', 'sk-...abcd');
           INSERT INTO "LiteLLM_TeamTable" ("team_id", "team_alias") VALUES ('t1', 'team alias');
           INSERT INTO "LiteLLM_UserTable" ("user_id", "user_email") VALUES ('u1', 'user@example.com')"#,
    )
    .await;
    let repo = repository(&pool);
    let keys = repo
        .label_rows(LabelKind::Key, &["k1".to_owned(), "gone".to_owned()])
        .await
        .unwrap();
    let teams = repo
        .label_rows(LabelKind::Team, &["t1".to_owned()])
        .await
        .unwrap();
    let users = repo
        .label_rows(LabelKind::User, &["u1".to_owned()])
        .await
        .unwrap();
    let empty = repo.label_rows(LabelKind::Key, &[]).await.unwrap();
    pool.close();
    fixture.close_async().await.unwrap();
    assert_label_rows(&keys, &teams, &users, &empty);
}

fn assert_label_rows(
    keys: &[LabelRow],
    teams: &[LabelRow],
    users: &[LabelRow],
    empty: &[LabelRow],
) {
    assert_eq!(
        keys,
        vec![LabelRow {
            target_id: "k1".to_owned(),
            target_alias: Some("key alias".to_owned()),
            key_name: Some("sk-...abcd".to_owned()),
        }]
    );
    assert_eq!(
        teams,
        vec![LabelRow {
            target_id: "t1".to_owned(),
            target_alias: Some("team alias".to_owned()),
            key_name: None,
        }]
    );
    assert_eq!(users.len(), 1);
    assert_eq!(
        users.first().unwrap().target_alias.as_deref(),
        Some("user@example.com")
    );
    assert!(empty.is_empty());
}

#[tokio::test]
async fn labeled_responses_read_each_kind_and_keep_its_label() {
    let (fixture, pool) = database().await;
    execute(&pool, JOBS).await;
    execute(
        &pool,
        r#"INSERT INTO "LiteLLM_VerificationToken" ("token", "key_alias", "key_name")
               VALUES ('k1', 'key alias', 'sk-...abcd');
           INSERT INTO "LiteLLM_TeamTable" ("team_id", "team_alias") VALUES ('t1', 'team alias')"#,
    )
    .await;
    let both = legs(&pool, &["leg-a", "leg-b"]).await;
    let group = group_response("job-1", &both, &LegCounts::new()).unwrap();
    let labeled = repository(&pool).labeled_responses(&[group]).await.unwrap();
    let empty = repository(&pool).labeled_responses(&[]).await.unwrap();
    pool.close();
    fixture.close_async().await.unwrap();
    let targets = &labeled.first().unwrap().targets;
    let key = targets.first().unwrap();
    assert_eq!(key.target_type, KeyTarget);
    assert_eq!(key.target_alias.as_deref(), Some("key alias"));
    assert_eq!(key.key_name.as_deref(), Some("sk-...abcd"));
    let team = targets.last().unwrap();
    assert_eq!(team.target_type, TeamTarget);
    assert_eq!(team.target_alias.as_deref(), Some("team alias"));
    assert_eq!(team.key_name, None);
    assert!(empty.is_empty());
}

#[tokio::test]
async fn leg_rows_read_naive_columns_as_utc() {
    let (fixture, pool) = database().await;
    execute(
        &pool,
        r#"INSERT INTO "LiteLLM_ShadowEvalJob"
            ("id", "group_id", "target_type", "target_id", "router_name", "router_names",
             "models", "direction", "baseline_model", "shadow_percentage", "judge_model",
             "max_turns", "max_budget", "created_at", "ends_at", "stopped_at", "stopped_by")
           VALUES
            ('leg-a', 'job-1', 'user', 'u1', 'router-a', ARRAY['router-a', 'router-b'],
             ARRAY['gpt-5.2'], 'reverse', 'gpt-5.2', 12.5, 'judge', 7, 2.5,
             '2026-11-21 12:00:00.123', '2026-11-28 12:00:00', '2026-11-22 08:00:00', 'alice')"#,
    )
    .await;
    let decoded = legs(&pool, &["leg-a"]).await;
    pool.close();
    fixture.close_async().await.unwrap();
    let job = decoded.first().unwrap();
    assert_job_config(job);
    assert_job_times(job);
}

fn assert_job_config(job: &LegRow) {
    assert_eq!(job.group_id, "job-1");
    assert_eq!(job.target_type, ShadowEvalTargetType::User);
    assert_eq!(job.direction, ShadowEvalDirection::Reverse);
    assert_eq!(job.router_names, vec!["router-a", "router-b"]);
    assert_eq!(job.models, vec!["gpt-5.2"]);
    assert_eq!(job.max_turns, 7);
    assert_eq!(job.max_budget, Some(Decimal::new(25, 1)));
    assert_eq!(job.stopped_by.as_deref(), Some("alice"));
}

fn assert_job_times(job: &LegRow) {
    let created: DateTime<Utc> = "2026-11-21T12:00:00.123Z".parse().unwrap();
    let ends: DateTime<Utc> = "2026-11-28T12:00:00Z".parse().unwrap();
    let stopped: DateTime<Utc> = "2026-11-22T08:00:00Z".parse().unwrap();
    assert_eq!(job.created_at, created);
    assert_eq!(job.ends_at, ends);
    assert_eq!(job.stopped_at, Some(stopped));
}

#[tokio::test]
async fn job_results_join_every_read_and_label_each_target() {
    let (fixture, pool) = database().await;
    execute(&pool, JOBS).await;
    execute(&pool, ATTEMPTS).await;
    execute(
        &pool,
        r#"INSERT INTO "LiteLLM_ShadowEvalFunnel" ("job_id") VALUES ('leg-a')"#,
    )
    .await;
    let both = legs(&pool, &["leg-a", "leg-b"]).await;
    let covered = repository(&pool).job_results(&both).await.unwrap();
    pool.close();
    fixture.close_async().await.unwrap();
    let result = covered.results.unwrap();
    assert_eq!(result.by_tier.len(), 2);
    assert_eq!(result.by_tier.first().unwrap().group, "simple");
    assert_eq!(result.not_sampled_count, None);
    assert_eq!(covered.verdicts.len(), 2);
    assert_eq!(verdict_group(&covered.verdicts, KeyTarget, "k1"), "k1");
    assert_eq!(verdict_group(&covered.verdicts, TeamTarget, "t1"), "t1");
}

#[tokio::test]
async fn job_results_carry_a_funnel_only_when_every_leg_reported() {
    let (fixture, pool) = database().await;
    execute(&pool, JOBS).await;
    execute(&pool, ATTEMPTS).await;
    execute(
        &pool,
        r#"INSERT INTO "LiteLLM_ShadowEvalFunnel" ("job_id") VALUES ('leg-a')"#,
    )
    .await;
    let one = legs(&pool, &["leg-a"]).await;
    let fully = repository(&pool).job_results(&one).await.unwrap();
    let none = repository(&pool).job_results(&[]).await.unwrap();
    pool.close();
    fixture.close_async().await.unwrap();
    assert_eq!(fully.results.unwrap().not_sampled_count, Some(0));
    assert_eq!(none.results, None);
    assert!(none.verdicts.is_empty());
}

#[tokio::test]
async fn a_database_error_names_its_source() {
    let (fixture, pool) = database().await;
    let repo = repository(&pool);
    execute(&pool, "DROP TABLE \"LiteLLM_ShadowEvalAttempt\"").await;
    let outcome = repo
        .attempt_aggregates(&["leg-a".to_owned()], GroupBy::Tier)
        .await;
    pool.close();
    fixture.close_async().await.unwrap();
    match outcome {
        Err(ShadowResultsError::Database(_)) => {}
        other => panic!("{other:?}"),
    }
}
