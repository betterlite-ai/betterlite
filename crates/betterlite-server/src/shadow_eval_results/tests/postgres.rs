#![cfg(test)]

use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;

use super::super::store::{ShadowLegReads as _, ShadowTargetFilter};
use crate::shadow_eval_job_loading::jobs::ShadowEvalDirection;
use crate::shadow_eval_models::ShadowEvalTargetType::{Key, Team};
use crate::shadow_result_aggregation::legs::LegRow;
use crate::shadow_result_aggregation::postgres::PgShadowResults;

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
"#;

const SEED: &str = r#"
INSERT INTO "LiteLLM_ShadowEvalJob"
    ("id", "group_id", "target_type", "target_id", "router_name", "router_names", "models",
     "direction", "baseline_model", "shadow_percentage", "judge_model", "max_turns",
     "max_budget", "created_at", "ends_at", "stopped_at", "stopped_by")
VALUES
    ('leg-a1', 'job-a', 'team', 't1', 'router-a', ARRAY['router-a'], ARRAY['gpt-5.2'],
     'forward', NULL, 10, 'judge', 100, 5, '2026-11-21 12:00:00', '2026-11-28 12:00:00', NULL, NULL),
    ('leg-a2', 'job-a', 'key', 'k1', 'router-a', ARRAY['router-a'], ARRAY['gpt-5.2'],
     'reverse', 'gpt-5.2', 25.5, 'judge', 50, NULL, '2026-11-21 13:00:00', '2026-11-28 12:00:00',
     '2026-11-22 08:00:00', 'alice'),
    ('leg-b1', 'job-b', 'user', 'u1', 'router-b', ARRAY['router-b'], ARRAY[]::TEXT[],
     'forward', NULL, 10, 'judge', 100, NULL, '2026-11-21 14:00:00', '2026-11-28 12:00:00', NULL, NULL),
    ('leg-c1', 'job-c', 'team', 't2', 'router-a', ARRAY['router-a'], ARRAY['gpt-5.2'],
     'forward', NULL, 10, 'judge', 100, NULL, '2026-11-21 11:00:00', '2026-11-28 12:00:00', NULL, NULL);
"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn database() -> (PostgresFixture, Pool) {
    let url = Environment::parse_from(["shadow-eval-results-tests"])
        .url
        .map(SecretString::from);
    let fixture = PostgresFixture::provision_with_statements(url.as_ref(), vec![SCHEMA.to_owned()])
        .resolve()
        .await
        .unwrap();
    let manager = Manager::new(fixture.options.url.expose_secret().parse().unwrap(), NoTls);
    let pool = Pool::builder(manager).max_size(1).build().unwrap();
    let client = pool.get().await.unwrap();
    client.batch_execute(SEED).await.unwrap();
    (fixture, pool)
}

fn store(pool: Pool) -> PgShadowResults {
    PgShadowResults {
        store: PostgresStore { pool: Some(pool) },
    }
}

async fn close(fixture: PostgresFixture, results: PgShadowResults) {
    if let Some(pool) = results.store.pool {
        pool.close();
    }
    fixture.close_async().await.unwrap();
}

fn ids(legs: &[LegRow]) -> Vec<&str> {
    let mut ids: Vec<&str> = legs.iter().map(|leg| leg.id.as_str()).collect();
    ids.sort_unstable();
    ids
}

#[tokio::test]
async fn list_legs_takes_the_newest_jobs_and_every_leg_of_each() {
    let (fixture, pool) = database().await;
    let results = store(pool);
    let newest = results.list_legs(2, None).await.unwrap();
    assert_eq!(ids(&newest), vec!["leg-a1", "leg-a2", "leg-b1"]);
    let all = results.list_legs(200, None).await.unwrap();
    assert_eq!(ids(&all), vec!["leg-a1", "leg-a2", "leg-b1", "leg-c1"]);
    let one = results.list_legs(1, None).await.unwrap();
    assert_eq!(ids(&one), vec!["leg-b1"]);
    close(fixture, results).await;
}

#[tokio::test]
async fn list_legs_filters_jobs_by_target_and_keeps_their_other_legs() {
    let (fixture, pool) = database().await;
    let results = store(pool);
    let team_filter = ShadowTargetFilter {
        target_type: Team,
        target_id: "t2".to_owned(),
    };
    let by_team = results.list_legs(50, Some(team_filter)).await.unwrap();
    assert_eq!(ids(&by_team), vec!["leg-c1"]);
    let key_filter = ShadowTargetFilter {
        target_type: Key,
        target_id: "k1".to_owned(),
    };
    let by_key = results.list_legs(50, Some(key_filter)).await.unwrap();
    assert_eq!(ids(&by_key), vec!["leg-a1", "leg-a2"]);
    close(fixture, results).await;
}

#[tokio::test]
async fn legs_of_group_decode_every_column_of_one_job() {
    let (fixture, pool) = database().await;
    let results = store(pool);
    let legs = results.legs_of_group("job-a").await.unwrap();
    assert_eq!(ids(&legs), vec!["leg-a1", "leg-a2"]);
    let key_leg = legs.iter().find(|leg| leg.id == "leg-a2").unwrap();
    assert_eq!(key_leg.direction, ShadowEvalDirection::Reverse);
    assert_eq!(key_leg.shadow_percentage.to_bits(), 25.5_f64.to_bits());
    assert_eq!(key_leg.max_turns, 50);
    assert_eq!(key_leg.max_budget, None);
    assert_eq!(key_leg.stopped_by.as_deref(), Some("alice"));
    assert!(key_leg.stopped_at.is_some());
    let missing = results.legs_of_group("job-none").await.unwrap();
    assert!(missing.is_empty());
    close(fixture, results).await;
}
