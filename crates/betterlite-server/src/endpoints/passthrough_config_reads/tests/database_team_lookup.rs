#![cfg(test)]

use std::sync::Arc;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::Manager;
use deadpool_postgres::Pool;
use secrecy::ExposeSecret as _;
use secrecy::SecretString;
use tokio_postgres::NoTls;

use crate::endpoints::passthrough_config_reads::ports::TeamLookup as _;
use crate::endpoints::passthrough_config_reads::support::json;
use crate::endpoints::passthrough_config_reads::team_lookup::DatabaseTeamLookup;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

const SCHEMA: &str = r#"
CREATE TABLE "LiteLLM_TeamTable" (
    team_id TEXT PRIMARY KEY,
    metadata JSONB
);
"#;

const INSERT_SQL: &str =
    r#"INSERT INTO "LiteLLM_TeamTable" (team_id, metadata) VALUES ($1, $2::text::jsonb)"#;

async fn database() -> (PostgresFixture, Pool) {
    let url = Environment::parse_from(["passthrough-config-reads-tests"])
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

fn lookup(pool: &Pool) -> DatabaseTeamLookup {
    DatabaseTeamLookup {
        store: Arc::new(PostgresStore {
            pool: Some(pool.clone()),
        }),
    }
}

async fn close(fixture: PostgresFixture, pool: Pool) {
    pool.close();
    fixture.close_async().await.unwrap();
}

async fn insert_team(pool: &Pool, team_id: &str, metadata: Option<&str>) {
    let client = pool.get().await.unwrap();
    client
        .execute(INSERT_SQL, &[&team_id, &metadata])
        .await
        .unwrap();
}

#[tokio::test]
async fn a_team_row_yields_its_metadata() {
    let (fixture, pool) = database().await;
    insert_team(
        &pool,
        "t1",
        Some(r#"{"allowed_passthrough_routes": ["/a"]}"#),
    )
    .await;
    let found = lookup(&pool).find_team_metadata("t1").await.unwrap();
    assert_eq!(
        found,
        Some(json(r#"{"allowed_passthrough_routes": ["/a"]}"#))
    );
    close(fixture, pool).await;
}

#[tokio::test]
async fn a_team_with_null_metadata_yields_null() {
    let (fixture, pool) = database().await;
    insert_team(&pool, "t1", None).await;
    let found = lookup(&pool).find_team_metadata("t1").await.unwrap();
    assert_eq!(found, Some(JsonData::Null));
    close(fixture, pool).await;
}

#[tokio::test]
async fn a_missing_team_yields_none() {
    let (fixture, pool) = database().await;
    insert_team(&pool, "other", Some("{}")).await;
    let found = lookup(&pool).find_team_metadata("t1").await.unwrap();
    assert_eq!(found, None);
    close(fixture, pool).await;
}
