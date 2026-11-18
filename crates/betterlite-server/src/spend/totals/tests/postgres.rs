use std::error::Error as _;

use chrono::NaiveDate;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

use betterlite_db::database_views::definitions::{
    LAST_30D_KEYS_BY_SPEND_SQL, LAST_30D_MODELS_BY_SPEND_SQL, MONTHLY_GLOBAL_SPEND_SQL,
    SPEND_PER_KEY_SQL, USER_KEY_SPEND_SQL,
};
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;

use super::super::store::{PostgresTotalsStore, TotalsStore as _, TotalsStoreError};
use super::support::{decimal, retained};

const SCHEMA_SQL: &str = r#"
CREATE TABLE "LiteLLM_SpendLogs" (
    request_id TEXT PRIMARY KEY,
    call_type TEXT NOT NULL DEFAULT '',
    api_key TEXT NOT NULL DEFAULT '',
    spend DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    total_tokens INTEGER NOT NULL DEFAULT 0,
    "startTime" TIMESTAMP(3) NOT NULL,
    "endTime" TIMESTAMP(3),
    model TEXT NOT NULL DEFAULT '',
    "user" TEXT DEFAULT '',
    team_id TEXT
);
CREATE TABLE "LiteLLM_VerificationToken" (
    token TEXT PRIMARY KEY,
    key_alias TEXT,
    key_name TEXT,
    user_id TEXT
);
CREATE TABLE "LiteLLM_TeamTable" (
    team_id TEXT PRIMARY KEY,
    team_alias TEXT
);
"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

fn postgres_store(pool: &Pool) -> PostgresTotalsStore {
    PostgresTotalsStore {
        store: PostgresStore {
            pool: Some(pool.clone()),
        },
    }
}

async fn database() -> (PostgresFixture, Pool) {
    let url = Environment::parse_from(["spend-totals-tests"])
        .url
        .map(SecretString::from);
    let views = format!(
        "{SCHEMA_SQL}\n{MONTHLY_GLOBAL_SPEND_SQL}\n{SPEND_PER_KEY_SQL}\n{USER_KEY_SPEND_SQL}\n{LAST_30D_KEYS_BY_SPEND_SQL}\n{LAST_30D_MODELS_BY_SPEND_SQL}"
    );
    let fixture = PostgresFixture::provision_with_statements(url.as_ref(), vec![views])
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

async fn current_day(pool: &Pool, offset: i32) -> NaiveDate {
    let client = pool.get().await.unwrap();
    let sql = format!("SELECT (CURRENT_DATE - INTERVAL '{offset} days')::date AS day");
    let row = client.query_one(&sql, &[]).await.unwrap();
    row.get("day")
}

#[tokio::test]
async fn reads_daily_spend_scoped_by_key_and_user() {
    let (fixture, pool) = database().await;
    execute(
        &pool,
        r#"
        INSERT INTO "LiteLLM_SpendLogs" (request_id, call_type, api_key, spend, "startTime", "endTime", model, "user", team_id, total_tokens) VALUES
            ('r1', 'completion', 'sk-one', 1.5, CURRENT_DATE - INTERVAL '1 day', CURRENT_DATE - INTERVAL '1 day', 'gpt-4o', 'u1', 't1', 10),
            ('r2', 'completion', 'sk-one', 0.25, CURRENT_DATE - INTERVAL '2 days', CURRENT_DATE - INTERVAL '2 days', 'gpt-4o', 'u1', 't1', 5),
            ('r3', 'completion', 'sk-two', 2.25, CURRENT_DATE - INTERVAL '1 day', CURRENT_DATE - INTERVAL '1 day', 'gpt-4o-mini', 'u2', 't2', 7);
        "#,
    )
    .await;
    let store = postgres_store(&pool);
    let global = store.global_daily_spend().await.unwrap();
    assert_eq!(global.len(), 2);
    assert_eq!(global[0].spend, decimal("0.25"));
    assert_eq!(global[1].spend, decimal("3.75"));
    let key = store.key_daily_spend("sk-one").await.unwrap();
    assert_eq!(key.len(), 2);
    assert_eq!(key[0].api_key, "sk-one");
    assert_eq!(key[1].spend, decimal("1.5"));
    let user = store.user_daily_spend("u1", None).await.unwrap();
    assert_eq!(user.len(), 2);
    assert_eq!(user[1].user, "u1");
    let scoped = store.user_daily_spend("u1", Some("sk-one")).await.unwrap();
    assert_eq!(scoped.len(), 2);
    assert!(
        store
            .user_daily_spend("u1", Some("sk-two"))
            .await
            .unwrap()
            .is_empty()
    );
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn sums_an_empty_view_to_a_null_total() {
    let (fixture, pool) = database().await;
    let store = postgres_store(&pool);
    let empty = store.global_total_spend().await.unwrap();
    assert_eq!(empty.len(), 1);
    assert_eq!(empty[0].total_spend, None);
    execute(
        &pool,
        r#"
        INSERT INTO "LiteLLM_SpendLogs" (request_id, call_type, api_key, spend, "startTime", "endTime", model, "user") VALUES
            ('r1', 'completion', 'sk-one', 0.1, CURRENT_DATE, CURRENT_DATE, 'gpt-4o', 'u1'),
            ('r2', 'completion', 'sk-one', 0.2, CURRENT_DATE, CURRENT_DATE, 'gpt-4o', 'u1');
        "#,
    )
    .await;
    let summed = store.global_total_spend().await.unwrap();
    assert_eq!(summed.len(), 1);
    assert_eq!(summed[0].total_spend, Some(decimal("0.30000000000000004")));
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn ranks_the_last_thirty_days_keys_and_models() {
    let (fixture, pool) = database().await;
    execute(
        &pool,
        r#"
        INSERT INTO "LiteLLM_SpendLogs" (request_id, call_type, api_key, spend, "startTime", "endTime", model, "user", total_tokens) VALUES
            ('r1', 'completion', 'sk-one', 1.5, CURRENT_DATE - INTERVAL '1 day', CURRENT_DATE - INTERVAL '1 day', 'gpt-4o', 'u1', 10),
            ('r2', 'completion', 'sk-two', 2.5, CURRENT_DATE - INTERVAL '2 days', CURRENT_DATE - INTERVAL '2 days', 'gpt-4o-mini', 'u1', 20),
            ('r3', 'completion', 'sk-one', 0.5, CURRENT_DATE - INTERVAL '45 days', CURRENT_DATE - INTERVAL '45 days', 'claude-3', 'u1', 30);
        INSERT INTO "LiteLLM_VerificationToken" (token, key_name, key_alias, user_id) VALUES
            ('sk-one', 'one', 'primary', 'u1'),
            ('sk-two', NULL, 'secondary', 'u1');
        "#,
    )
    .await;
    let store = postgres_store(&pool);
    let keys = store.keys_by_spend(None).await.unwrap();
    assert_eq!(keys.len(), 2);
    assert_eq!(keys[0].api_key, "sk-two");
    assert_eq!(keys[0].total_spend, decimal("2.5"));
    assert_eq!(keys[0].key_alias.as_deref(), Some("secondary"));
    assert_eq!(keys[0].key_name, None);
    assert_eq!(keys[1].api_key, "sk-one");
    assert_eq!(keys[1].total_spend, decimal("1.5"));
    assert_eq!(keys[1].key_name.as_deref(), Some("one"));
    let limited = store.keys_by_spend(Some(1)).await.unwrap();
    assert_eq!(limited.len(), 1);
    assert_eq!(limited[0].api_key, "sk-two");
    let models = store.models_by_spend(10).await.unwrap();
    assert_eq!(models.len(), 2);
    assert_eq!(models[0].model, "gpt-4o-mini");
    assert_eq!(models[1].total_spend, decimal("1.5"));
    let top_model = store.models_by_spend(1).await.unwrap();
    assert_eq!(top_model.len(), 1);
    assert_eq!(top_model[0].model, "gpt-4o-mini");
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn aggregates_a_users_full_history() {
    let (fixture, pool) = database().await;
    execute(
        &pool,
        r#"
        INSERT INTO "LiteLLM_SpendLogs" (request_id, call_type, api_key, spend, "startTime", "endTime", model, "user", total_tokens) VALUES
            ('r1', 'completion', 'sk-one', 1.5, CURRENT_DATE - INTERVAL '1 day', CURRENT_DATE - INTERVAL '1 day', 'gpt-4o', 'u1', 10),
            ('r2', 'completion', 'sk-two', 2.5, CURRENT_DATE - INTERVAL '2 days', CURRENT_DATE - INTERVAL '2 days', 'gpt-4o-mini', 'u1', 20),
            ('r3', 'completion', 'sk-one', 0.5, CURRENT_DATE - INTERVAL '45 days', CURRENT_DATE - INTERVAL '45 days', 'claude-3', 'u1', 30),
            ('r4', 'completion', 'sk-nine', 9.0, CURRENT_DATE - INTERVAL '40 days', CURRENT_DATE - INTERVAL '40 days', 'u2-model', 'u2', 40);
        INSERT INTO "LiteLLM_VerificationToken" (token, key_name, key_alias, user_id) VALUES
            ('sk-one', 'one', 'primary', 'u1'),
            ('sk-two', NULL, 'secondary', 'u1');
        "#,
    )
    .await;
    let store = postgres_store(&pool);
    let models = store.user_models_by_spend("u1", 10).await.unwrap();
    assert_eq!(models.len(), 3);
    assert_eq!(models[0].model, "gpt-4o-mini");
    assert_eq!(models[0].total_tokens, 20);
    assert_eq!(models[2].model, "claude-3");
    assert_eq!(models[2].total_tokens, 30);
    assert_eq!(
        store.user_models_by_spend("u2", 10).await.unwrap()[0].model,
        "u2-model"
    );
    assert_eq!(store.user_models_by_spend("u1", 1).await.unwrap().len(), 1);
    let keys = store.user_keys_by_spend("u1", 10).await.unwrap();
    assert_eq!(keys.len(), 2);
    assert_eq!(keys[0].api_key, "sk-two");
    assert_eq!(keys[0].total_spend, decimal("2.5"));
    assert_eq!(keys[0].key_alias.as_deref(), Some("secondary"));
    assert_eq!(keys[1].api_key, "sk-one");
    assert_eq!(keys[1].total_spend, decimal("2"));
    assert_eq!(keys[1].key_name.as_deref(), Some("one"));
    let limited = store.user_keys_by_spend("u1", 1).await.unwrap();
    assert_eq!(limited.len(), 1);
    assert_eq!(limited[0].api_key, "sk-two");
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn groups_team_spend_by_alias_and_day() {
    let (fixture, pool) = database().await;
    execute(
        &pool,
        r#"
        INSERT INTO "LiteLLM_TeamTable" (team_id, team_alias) VALUES ('t1', 'alpha'), ('t2', NULL);
        INSERT INTO "LiteLLM_SpendLogs" (request_id, call_type, api_key, spend, "startTime", "endTime", model, "user", team_id) VALUES
            ('r1', 'completion', 'sk-one', 0.1, CURRENT_DATE - INTERVAL '1 day', CURRENT_DATE - INTERVAL '1 day', 'gpt-4o', 'u1', 't1'),
            ('r2', 'completion', 'sk-one', 0.2, CURRENT_DATE - INTERVAL '1 day', CURRENT_DATE - INTERVAL '1 day', 'gpt-4o', 'u1', 't1'),
            ('r3', 'completion', 'sk-one', 1.0, CURRENT_DATE - INTERVAL '1 day', CURRENT_DATE - INTERVAL '1 day', 'gpt-4o', 'u1', 't2'),
            ('r4', 'completion', 'sk-one', 2.0, CURRENT_DATE - INTERVAL '1 day', CURRENT_DATE - INTERVAL '1 day', 'gpt-4o', 'u1', NULL),
            ('r5', 'completion', 'sk-one', 5.0, CURRENT_DATE - INTERVAL '40 days', CURRENT_DATE - INTERVAL '40 days', 'gpt-4o', 'u1', 't1');
        "#,
    )
    .await;
    let expected_day = current_day(&pool, 1).await;
    let store = postgres_store(&pool);
    let rows = store.team_daily_spend().await.unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|row| row.spend_date == Some(expected_day)));
    let alpha = rows
        .iter()
        .find(|row| row.team_alias.as_deref() == Some("alpha"))
        .unwrap();
    let unassigned = rows.iter().find(|row| row.team_alias.is_none()).unwrap();
    assert_eq!(alpha.total_spend, Some(retained(0.300_000_000_000_000_04)));
    assert_eq!(unassigned.total_spend, Some(decimal("3")));
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn keeps_the_parse_error_of_an_unreadable_amount() {
    let (fixture, pool) = database().await;
    execute(
        &pool,
        r#"
        INSERT INTO "LiteLLM_SpendLogs" (request_id, call_type, api_key, spend, "startTime", "endTime", model, "user") VALUES
            ('r1', 'completion', 'sk-one', 'NaN', CURRENT_DATE, CURRENT_DATE, 'gpt-4o', 'u1');
        "#,
    )
    .await;
    let store = postgres_store(&pool);
    let total = store.global_total_spend().await.unwrap_err();
    assert!(total.source().is_some());
    assert!(matches!(
        total,
        TotalsStoreError::UnreadableAmount {
            column: "spend_text",
            ..
        }
    ));
    let daily = store.global_daily_spend().await.unwrap_err();
    assert!(matches!(
        daily,
        TotalsStoreError::UnreadableAmount {
            column: "spend",
            ..
        }
    ));
    let teams = store.team_daily_spend().await.unwrap_err();
    assert!(matches!(
        teams,
        TotalsStoreError::InvalidAmount {
            column: "total_spend"
        }
    ));
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn reads_an_exponent_amount_through_the_scientific_fallback() {
    let (fixture, pool) = database().await;
    execute(
        &pool,
        r#"
        INSERT INTO "LiteLLM_SpendLogs" (request_id, call_type, api_key, spend, "startTime", "endTime", model, "user") VALUES
            ('r1', 'completion', 'sk-one', 1e20, CURRENT_DATE, CURRENT_DATE, 'gpt-4o', 'u1');
        "#,
    )
    .await;
    let store = postgres_store(&pool);
    let total = store.global_total_spend().await.unwrap();
    assert_eq!(total[0].total_spend, Some(decimal("100000000000000000000")));
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn reports_no_database_without_a_pool() {
    let store = PostgresTotalsStore {
        store: PostgresStore { pool: None },
    };
    let missing = |result: Result<(), TotalsStoreError>| {
        assert!(matches!(
            result,
            Err(TotalsStoreError::Database(DbError::NoDbConnected))
        ));
    };
    missing(store.global_daily_spend().await.map(drop));
    missing(store.key_daily_spend("sk-one").await.map(drop));
    missing(store.user_daily_spend("u1", None).await.map(drop));
    missing(store.global_total_spend().await.map(drop));
    missing(store.keys_by_spend(None).await.map(drop));
    missing(store.user_keys_by_spend("u1", 10).await.map(drop));
    missing(store.models_by_spend(10).await.map(drop));
    missing(store.user_models_by_spend("u1", 10).await.map(drop));
    missing(store.team_daily_spend().await.map(drop));
}
