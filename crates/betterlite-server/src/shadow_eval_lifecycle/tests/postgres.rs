#![cfg(test)]

use chrono::{DateTime, TimeZone as _, Utc};
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;

use crate::shadow_eval_job_loading::jobs::ShadowEvalDirection;
use crate::shadow_eval_lifecycle::postgres::sql::{INSERT_LEGS_HEAD, insert_legs};
use crate::shadow_eval_lifecycle::postgres::{
    PostgresLifecycleError, PostgresShadowLifecycleStore,
};
use crate::shadow_eval_lifecycle::store::{InsertRejection, NewLeg, ShadowLifecycleStore as _};
use crate::shadow_eval_models::ShadowEvalTargetType;
use crate::shadow_eval_models::ShadowEvalTargetType::{
    Key as KeyTarget, Team as TeamTarget, User as UserTarget,
};

const SCHEMA: &str = r#"
CREATE TABLE "LiteLLM_ShadowEvalJob" (
    "id" TEXT PRIMARY KEY,
    "group_id" TEXT NOT NULL,
    "target_type" TEXT NOT NULL DEFAULT 'key',
    "target_id" TEXT NOT NULL,
    "router_name" TEXT NOT NULL,
    "router_names" TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[],
    "models" TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[],
    "direction" TEXT NOT NULL DEFAULT 'forward',
    "baseline_model" TEXT,
    "judge_model" TEXT NOT NULL,
    "shadow_percentage" DOUBLE PRECISION NOT NULL,
    "max_turns" INTEGER NOT NULL,
    "max_budget" DOUBLE PRECISION,
    "created_at" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "created_by" TEXT,
    "ends_at" TIMESTAMP(3) NOT NULL,
    "stopped_at" TIMESTAMP(3),
    "stopped_by" TEXT
);
CREATE UNIQUE INDEX "LiteLLM_ShadowEvalJob_one_active_per_target_direction"
    ON "LiteLLM_ShadowEvalJob"("target_type", "target_id", "direction") WHERE "stopped_at" IS NULL;
CREATE TABLE "LiteLLM_ShadowEvalAttempt" (
    "id" TEXT PRIMARY KEY,
    "job_id" TEXT NOT NULL,
    "request_id" TEXT NOT NULL,
    "outcome" TEXT NOT NULL,
    "judge_cost" DOUBLE PRECISION NOT NULL DEFAULT 0,
    "shadow_cost" DOUBLE PRECISION NOT NULL DEFAULT 0,
    "shadow_classifier_cost" DOUBLE PRECISION NOT NULL DEFAULT 0,
    "created_at" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE "LiteLLM_ShadowEvalFunnel" (
    "job_id" TEXT PRIMARY KEY
);
CREATE TABLE "LiteLLM_VerificationToken" (
    "token" TEXT PRIMARY KEY,
    "key_alias" TEXT,
    "key_name" TEXT,
    "team_id" TEXT
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

const TARGETS: &str = r#"INSERT INTO "LiteLLM_VerificationToken" ("token", "key_alias", "key_name", "team_id")
VALUES ('k1', 'alias-one', 'name-one', 'team-1'), ('k2', NULL, NULL, NULL);
INSERT INTO "LiteLLM_TeamTable" ("team_id", "team_alias") VALUES ('team-1', 'Team One');
INSERT INTO "LiteLLM_UserTable" ("user_id", "user_email") VALUES ('user-1', 'user@example.com')"#;

const SWEEP_SEED: &str = r#"INSERT INTO "LiteLLM_ShadowEvalJob"
    ("id", "group_id", "target_type", "target_id", "router_name", "judge_model",
     "shadow_percentage", "max_turns", "max_budget", "direction", "ends_at", "stopped_at")
VALUES
    ('l-expired', 'g-expired', 'key', 'k1', 'r', 'j', 10, 10, NULL, 'forward', '2020-01-01 00:00:00', NULL),
    ('l-reverse', 'g-reverse', 'key', 'k2', 'r', 'j', 10, 10, NULL, 'reverse', '2020-01-01 00:00:00', NULL),
    ('l-turned', 'g-turned', 'key', 'k3', 'r', 'j', 10, 1, NULL, 'forward', '2030-01-01 00:00:00', NULL),
    ('l-spent', 'g-spent', 'key', 'k4', 'r', 'j', 10, 10, 0.5, 'forward', '2030-01-01 00:00:00', NULL),
    ('l-open', 'g-open', 'key', 'k5', 'r', 'j', 10, 10, NULL, 'forward', '2030-01-01 00:00:00', NULL),
    ('l-other', 'g-other', 'team', 'k6', 'r', 'j', 10, 10, NULL, 'forward', '2020-01-01 00:00:00', NULL),
    ('l-done', 'g-done', 'key', 'k7', 'r', 'j', 10, 10, NULL, 'forward', '2020-01-01 00:00:00', '2025-01-01 00:00:00');
INSERT INTO "LiteLLM_ShadowEvalAttempt"
    ("id", "job_id", "request_id", "outcome", "judge_cost", "shadow_cost", "shadow_classifier_cost")
VALUES
    ('a1', 'l-turned', 'r1', 'real', 0, 0, 0),
    ('a2', 'l-spent', 'r2', 'real', 0.25, 0.25, 0)"#;

const CLAIM_SEED: &str = r#"INSERT INTO "LiteLLM_ShadowEvalJob"
    ("id", "group_id", "target_type", "target_id", "router_name", "judge_model",
     "shadow_percentage", "max_turns", "direction", "ends_at", "stopped_at")
VALUES
    ('c1', 'g-key', 'key', 'k1', 'r', 'j', 10, 10, 'forward', '2030-01-01 00:00:00', NULL),
    ('c2', 'g-team', 'team', 't1', 'r', 'j', 10, 10, 'forward', '2030-01-01 00:00:00', NULL),
    ('c3', 'g-user', 'user', 'u1', 'r', 'j', 10, 10, 'forward', '2030-01-01 00:00:00', NULL),
    ('c4', 'g-reverse', 'key', 'k1', 'r', 'j', 10, 10, 'reverse', '2030-01-01 00:00:00', NULL),
    ('c5', 'g-stopped', 'key', 'k9', 'r', 'j', 10, 10, 'forward', '2030-01-01 00:00:00', '2026-01-01 00:00:00')"#;

const ATTEMPT_SEED: &str = r#"INSERT INTO "LiteLLM_ShadowEvalJob"
    ("id", "group_id", "target_type", "target_id", "router_name", "judge_model",
     "shadow_percentage", "max_turns", "direction", "ends_at", "stopped_at", "stopped_by")
VALUES
    ('leg-a', 'job-a', 'key', 'k1', 'r', 'j', 10, 10, 'forward', '2030-01-01 00:00:00', '2026-01-01 00:00:03', 'operator'),
    ('leg-b', 'job-b', 'team', 't1', 'r', 'j', 10, 10, 'forward', '2030-01-01 00:00:00', NULL, NULL);
INSERT INTO "LiteLLM_ShadowEvalAttempt"
    ("id", "job_id", "request_id", "outcome", "judge_cost", "shadow_cost", "shadow_classifier_cost", "created_at")
VALUES
    ('a1', 'leg-a', 'r1', 'real', 0.25, 0.25, 0, '2026-01-01 00:00:01'),
    ('a2', 'leg-a', 'r2', 'real', 0, 0, 0.25, '2026-01-01 00:00:02'),
    ('a3', 'leg-a', 'r3', 'real', 1, 0, 0, '2026-01-01 00:00:04'),
    ('b1', 'leg-b', 'r4', 'real', 0, 0, 0, '2026-01-01 00:00:01'),
    ('b2', 'leg-b', 'r5', 'real', 0.5, 0.25, 0.125, '2026-01-01 00:00:02')"#;

const STOP_SEED: &str = r#"INSERT INTO "LiteLLM_ShadowEvalJob"
    ("id", "group_id", "target_type", "target_id", "router_name", "judge_model",
     "shadow_percentage", "max_turns", "max_budget", "direction", "ends_at", "stopped_at", "stopped_by")
VALUES
    ('s1', 'g-sampling', 'key', 'k1', 'r', 'j', 10, 10, NULL, 'forward', '2030-01-01 00:00:00', NULL, NULL),
    ('c1', 'g-closed', 'key', 'k2', 'r', 'j', 10, 10, NULL, 'forward', '2020-01-01 00:00:00', NULL, NULL),
    ('t1', 'g-turned', 'key', 'k3', 'r', 'j', 10, 1, NULL, 'forward', '2030-01-01 00:00:00', NULL, NULL),
    ('p1', 'g-spent', 'key', 'k4', 'r', 'j', 10, 10, 0.5, 'forward', '2030-01-01 00:00:00', NULL, NULL),
    ('v1', 'g-even', 'key', 'k5', 'r', 'j', 10, 10, 0.5, 'forward', '2030-01-01 00:00:00', NULL, NULL),
    ('e1', 'g-earned', 'key', 'k6', 'r', 'j', 10, 10, NULL, 'forward', '2030-01-01 00:00:00', '2026-01-01 00:00:00', NULL),
    ('e2', 'g-earned', 'team', 't6', 'r', 'j', 10, 10, NULL, 'forward', '2030-01-01 00:00:00', NULL, NULL);
INSERT INTO "LiteLLM_ShadowEvalAttempt"
    ("id", "job_id", "request_id", "outcome", "judge_cost", "shadow_cost", "shadow_classifier_cost")
VALUES
    ('a1', 't1', 'r1', 'real', 0, 0, 0),
    ('a2', 'p1', 'r2', 'real', 0.5, 0, 0),
    ('a3', 'v1', 'r3', 'real', 0.25, 0, 0),
    ('a4', 'v1', 'r4', 'real', 0.25, 0, 0)"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

fn at(year: i32, month: u32, day: u32, hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(year, month, day, hour, 0, 0).unwrap()
}

fn lifecycle(pool: &Pool) -> PostgresShadowLifecycleStore {
    PostgresShadowLifecycleStore {
        store: PostgresStore {
            pool: Some(pool.clone()),
        },
    }
}

async fn database() -> (PostgresFixture, Pool) {
    let url = Environment::parse_from(["shadow-eval-lifecycle-tests"])
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

async fn groups_stamped(pool: &Pool) -> Vec<String> {
    let client = pool.get().await.unwrap();
    let rows = client
        .query(
            r#"SELECT "group_id" FROM "LiteLLM_ShadowEvalJob"
               WHERE "stopped_at" IS NOT NULL ORDER BY "group_id""#,
            &[],
        )
        .await
        .unwrap();
    rows.iter()
        .map(|row| row.get::<_, String>("group_id"))
        .collect()
}

async fn funnel_ids(pool: &Pool) -> Vec<String> {
    let client = pool.get().await.unwrap();
    let rows = client
        .query(
            r#"SELECT "job_id" FROM "LiteLLM_ShadowEvalFunnel" ORDER BY "job_id""#,
            &[],
        )
        .await
        .unwrap();
    rows.iter()
        .map(|row| row.get::<_, String>("job_id"))
        .collect()
}

async fn created_by_of(pool: &Pool, id: &str) -> Option<String> {
    let client = pool.get().await.unwrap();
    let row = client
        .query_one(
            r#"SELECT "created_by" FROM "LiteLLM_ShadowEvalJob" WHERE "id" = $1"#,
            &[&id],
        )
        .await
        .unwrap();
    row.get("created_by")
}

fn leg(id: &str, kind: ShadowEvalTargetType, target: &str) -> NewLeg {
    NewLeg {
        id: id.to_owned(),
        group_id: "job-1".to_owned(),
        target_type: kind,
        target_id: target.to_owned(),
        router_name: "shadow-router".to_owned(),
        router_names: vec!["shadow-router".to_owned(), "second-router".to_owned()],
        models: vec!["gpt-5.2".to_owned()],
        direction: ShadowEvalDirection::Forward,
        baseline_model: None,
        judge_model: "judge-model".to_owned(),
        shadow_percentage: 25.0,
        max_turns: 10_000,
        max_budget: Decimal::new(150, 2),
        created_by: Some("admin-user".to_owned()),
        created_at: at(2026, 11, 23, 12),
        ends_at: at(2026, 12, 23, 12),
    }
}

fn inserted(result: Result<(), InsertRejection<PostgresLifecycleError>>) {
    result.unwrap_or_else(|rejection| match rejection {
        InsertRejection::UniqueViolation => panic!("a fresh job hit the active-target index"),
        InsertRejection::Database(error) => panic!("a fresh job must insert: {error}"),
    });
}

#[tokio::test]
async fn load_targets_reads_each_requested_kind() {
    let (fixture, pool) = database().await;
    execute(&pool, TARGETS).await;
    let store = lifecycle(&pool);
    let loaded = store
        .load_targets(
            &["k1".to_owned()],
            &["team-1".to_owned()],
            &["user-1".to_owned()],
        )
        .await
        .unwrap();
    let absent = store
        .load_targets(&["missing".to_owned()], &[], &[])
        .await
        .unwrap();
    pool.close();
    fixture.close_async().await.unwrap();
    assert_eq!(loaded.keys.len(), 1);
    let key = loaded.keys.first().unwrap();
    assert_eq!(key.token, "k1");
    assert_eq!(key.key_alias.as_deref(), Some("alias-one"));
    assert_eq!(key.key_name.as_deref(), Some("name-one"));
    assert_eq!(key.team_id.as_deref(), Some("team-1"));
    assert_eq!(loaded.teams.len(), 1);
    let team = loaded.teams.first().unwrap();
    assert_eq!(team.team_id, "team-1");
    assert_eq!(team.team_alias.as_deref(), Some("Team One"));
    assert_eq!(loaded.users.len(), 1);
    let user = loaded.users.first().unwrap();
    assert_eq!(user.user_id, "user-1");
    assert_eq!(user.user_email.as_deref(), Some("user@example.com"));
    assert!(absent.keys.is_empty());
    assert!(absent.teams.is_empty());
    assert!(absent.users.is_empty());
}

#[tokio::test]
async fn sweep_finished_stamps_every_finished_leg_and_skips_a_stopped_one() {
    let (fixture, pool) = database().await;
    execute(&pool, SWEEP_SEED).await;
    let store = lifecycle(&pool);
    store
        .sweep_finished(
            &[
                "k1".to_owned(),
                "k2".to_owned(),
                "k3".to_owned(),
                "k4".to_owned(),
                "k5".to_owned(),
                "k6".to_owned(),
                "k7".to_owned(),
            ],
            KeyTarget,
        )
        .await
        .unwrap();
    let stamped = groups_stamped(&pool).await;
    let open = store.legs("g-open").await.unwrap();
    let done = store.legs("g-done").await.unwrap();
    pool.close();
    fixture.close_async().await.unwrap();
    assert_eq!(
        stamped,
        ["g-done", "g-expired", "g-reverse", "g-spent", "g-turned"]
    );
    assert_eq!(open.first().unwrap().stopped_at, None);
    assert_eq!(done.first().unwrap().stopped_at, Some(at(2025, 1, 1, 0)));
}

#[tokio::test]
async fn claimed_targets_reports_the_kind_direction_and_group() {
    let (fixture, pool) = database().await;
    execute(&pool, CLAIM_SEED).await;
    let store = lifecycle(&pool);
    let mut claimed = store
        .claimed_targets(
            &[
                (KeyTarget, vec!["k1".to_owned(), "k9".to_owned()]),
                (TeamTarget, vec!["t1".to_owned()]),
                (UserTarget, vec!["u1".to_owned()]),
            ],
            ShadowEvalDirection::Forward,
        )
        .await
        .unwrap();
    let other_kind = store
        .claimed_targets(
            &[(KeyTarget, vec!["u1".to_owned()])],
            ShadowEvalDirection::Forward,
        )
        .await
        .unwrap();
    let reverse = store
        .claimed_targets(
            &[(KeyTarget, vec!["k1".to_owned()])],
            ShadowEvalDirection::Reverse,
        )
        .await
        .unwrap();
    claimed.sort_by(|left, right| left.group_id.cmp(&right.group_id));
    pool.close();
    fixture.close_async().await.unwrap();
    let groups: Vec<&str> = claimed.iter().map(|row| row.group_id.as_str()).collect();
    assert_eq!(groups, ["g-key", "g-team", "g-user"]);
    let kinds: Vec<ShadowEvalTargetType> = claimed.iter().map(|row| row.target_type).collect();
    assert_eq!(kinds, [KeyTarget, TeamTarget, UserTarget]);
    let targets: Vec<&str> = claimed.iter().map(|row| row.target_id.as_str()).collect();
    assert_eq!(targets, ["k1", "t1", "u1"]);
    assert!(other_kind.is_empty());
    let reverse_groups: Vec<&str> = reverse.iter().map(|row| row.group_id.as_str()).collect();
    assert_eq!(reverse_groups, ["g-reverse"]);
}

#[tokio::test]
async fn create_legs_writes_every_column_and_rejects_a_second_claim() {
    let (fixture, pool) = database().await;
    let mut key = leg("leg-a", KeyTarget, "k1");
    key.direction = ShadowEvalDirection::Reverse;
    key.baseline_model = Some("baseline-model".to_owned());
    let mut team = leg("leg-b", TeamTarget, "t1");
    team.router_name = "legacy-router".to_owned();
    team.router_names = Vec::new();
    team.models = Vec::new();
    team.created_by = None;
    team.shadow_percentage = 12.5;
    team.max_turns = 3;
    team.max_budget = Decimal::new(2, 0);
    let user = leg("leg-c", UserTarget, "u1");
    let store = lifecycle(&pool);
    inserted(store.create_legs(&[key, team, user]).await);
    let mut read = store.legs("job-1").await.unwrap();
    read.sort_by(|left, right| left.id.cmp(&right.id));
    let mut duplicate = leg("leg-dup", KeyTarget, "k1");
    duplicate.direction = ShadowEvalDirection::Reverse;
    let rejected = store.create_legs(&[duplicate]).await;
    let after = store.legs("job-1").await.unwrap();
    let written_by = created_by_of(&pool, "leg-a").await;
    let team_by = created_by_of(&pool, "leg-b").await;
    pool.close();
    fixture.close_async().await.unwrap();
    let ids: Vec<&str> = read.iter().map(|row| row.id.as_str()).collect();
    assert_eq!(ids, ["leg-a", "leg-b", "leg-c"]);
    let written = read.first().unwrap();
    assert_eq!(written.group_id, "job-1");
    assert_eq!(written.target_type, KeyTarget);
    assert_eq!(written.target_id, "k1");
    assert_eq!(written.router_name, "shadow-router");
    assert_eq!(
        written.router_names,
        vec!["shadow-router".to_owned(), "second-router".to_owned()]
    );
    assert_eq!(written.models, vec!["gpt-5.2".to_owned()]);
    assert_eq!(written.direction, ShadowEvalDirection::Reverse);
    assert_eq!(written.baseline_model.as_deref(), Some("baseline-model"));
    assert_eq!(written.judge_model, "judge-model");
    assert_eq!(written.shadow_percentage, 25.0_f64);
    assert_eq!(written.max_turns, 10_000);
    assert_eq!(written.max_budget, Some(Decimal::new(15, 1)));
    assert_eq!(written_by.as_deref(), Some("admin-user"));
    assert_eq!(written.created_at, at(2026, 11, 23, 12));
    assert_eq!(written.ends_at, at(2026, 12, 23, 12));
    assert_eq!(written.stopped_at, None);
    assert_eq!(written.stopped_by, None);
    let written_team = read.get(1).unwrap();
    assert_eq!(written_team.target_type, TeamTarget);
    assert_eq!(written_team.direction, ShadowEvalDirection::Forward);
    assert_eq!(written_team.shadow_percentage, 12.5_f64);
    assert_eq!(written_team.max_turns, 3);
    assert_eq!(written_team.max_budget, Some(Decimal::new(2, 0)));
    assert!(written_team.router_names.is_empty());
    assert_eq!(
        written_team.arm_router_names(),
        vec!["legacy-router".to_owned()]
    );
    assert_eq!(team_by, None);
    rejected.unwrap_or_else(|rejection| match rejection {
        InsertRejection::UniqueViolation => {}
        InsertRejection::Database(error) => {
            panic!("the duplicate must lose the unique index, not fail: {error}")
        }
    });
    assert_eq!(after.len(), 3);
}

#[tokio::test]
async fn seed_funnel_keeps_one_row_per_leg() {
    let (fixture, pool) = database().await;
    let store = lifecycle(&pool);
    let legs = [
        leg("leg-a", KeyTarget, "k1"),
        leg("leg-b", TeamTarget, "t1"),
    ];
    inserted(store.create_legs(&legs).await);
    let ids = ["leg-a".to_owned(), "leg-b".to_owned()];
    store.seed_funnel(&ids).await.unwrap();
    store.seed_funnel(&ids).await.unwrap();
    let seeded = funnel_ids(&pool).await;
    pool.close();
    fixture.close_async().await.unwrap();
    assert_eq!(seeded, ["leg-a", "leg-b"]);
}

#[tokio::test]
async fn attempt_stats_count_only_attempts_before_the_stamp() {
    let (fixture, pool) = database().await;
    execute(&pool, ATTEMPT_SEED).await;
    let mut stats = lifecycle(&pool)
        .attempt_stats(&["leg-a".to_owned(), "leg-b".to_owned()])
        .await
        .unwrap();
    stats.sort_by(|left, right| left.job_id.cmp(&right.job_id));
    pool.close();
    fixture.close_async().await.unwrap();
    assert_eq!(stats.len(), 2);
    let stopped = stats.first().unwrap();
    assert_eq!(stopped.job_id, "leg-a");
    assert_eq!(stopped.attempt_count, 2);
    assert_eq!(stopped.spend, Decimal::new(75, 2));
    let open = stats.get(1).unwrap();
    assert_eq!(open.job_id, "leg-b");
    assert_eq!(open.attempt_count, 2);
    assert_eq!(open.spend, Decimal::new(875, 3));
}

#[tokio::test]
async fn stop_job_claims_only_a_leg_still_sampling_inside_its_window() {
    let (fixture, pool) = database().await;
    execute(&pool, STOP_SEED).await;
    let store = lifecycle(&pool);
    let sampling_stamp = at(2026, 11, 24, 9);
    let later_stamp = at(2026, 11, 25, 9);
    let earned_stamp = at(2026, 11, 24, 10);
    let first = store
        .stop_job("g-sampling", "admin-user", sampling_stamp)
        .await
        .unwrap();
    let second = store
        .stop_job("g-sampling", "admin-user", later_stamp)
        .await
        .unwrap();
    let closed = store
        .stop_job("g-closed", "admin-user", earned_stamp)
        .await
        .unwrap();
    let turned = store
        .stop_job("g-turned", "admin-user", earned_stamp)
        .await
        .unwrap();
    let spent = store
        .stop_job("g-spent", "admin-user", earned_stamp)
        .await
        .unwrap();
    let even = store
        .stop_job("g-even", "admin-user", earned_stamp)
        .await
        .unwrap();
    let earned = store
        .stop_job("g-earned", "admin-user", earned_stamp)
        .await
        .unwrap();
    let sampling = store.legs("g-sampling").await.unwrap();
    let mut group = store.legs("g-earned").await.unwrap();
    group.sort_by(|left, right| left.id.cmp(&right.id));
    let turned_legs = store.legs("g-turned").await.unwrap();
    pool.close();
    fixture.close_async().await.unwrap();
    assert_eq!(first, 1);
    assert_eq!(second, 0);
    assert_eq!(closed, 0);
    assert_eq!(turned, 0);
    assert_eq!(spent, 0);
    assert_eq!(even, 0);
    assert_eq!(earned, 2);
    let stamped = sampling.first().unwrap();
    assert_eq!(stamped.stopped_at, Some(sampling_stamp));
    assert_eq!(stamped.stopped_by.as_deref(), Some("admin-user"));
    let swept = group.first().unwrap();
    assert_eq!(swept.stopped_at, Some(at(2026, 1, 1, 0)));
    assert_eq!(swept.stopped_by.as_deref(), Some("admin-user"));
    let fresh = group.get(1).unwrap();
    assert_eq!(fresh.stopped_at, Some(earned_stamp));
    assert_eq!(fresh.stopped_by.as_deref(), Some("admin-user"));
    let left_alone = turned_legs.first().unwrap();
    assert_eq!(left_alone.stopped_at, None);
    assert_eq!(left_alone.stopped_by, None);
}

#[test]
fn insert_legs_joins_rows_and_casts_only_the_budget() {
    let one = format!(
        "{INSERT_LEGS_HEAD}($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13::numeric::float8, \
         $14, $15, $16)"
    );
    assert_eq!(insert_legs(1), one);
    let two = format!(
        "{INSERT_LEGS_HEAD}($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13::numeric::float8, \
         $14, $15, $16), ($17, $18, $19, $20, $21, $22, $23, $24, $25, $26, $27, $28, \
         $29::numeric::float8, $30, $31, $32)"
    );
    assert_eq!(insert_legs(2), two);
}
