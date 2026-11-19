#![cfg(test)]

use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser as _;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

use super::PostgresGatewayRequests;
use super::commit_gateway_requests_to_db;
use crate::gateway_request_tracking::test_support::FakeDatabase;
use crate::gateway_request_tracking::test_support::captured_logs;
use crate::gateway_request_tracking::test_support::snapshot;
use crate::gateway_requests::GatewayRequestSnapshot;
use crate::migration_ledger::baseline::{PACKAGED_CATALOGS, catalog_statements};

const STORED_ROWS: &str = r#"SELECT "date", "category", "route", "successful_requests", "failed_requests" FROM "LiteLLM_DailyGatewayRequests" ORDER BY "date""#;

#[derive(clap::Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn database() -> (PostgresFixture, Pool) {
    let url = Environment::parse_from(["gateway-request-tests"])
        .url
        .map(SecretString::from);
    let statements = catalog_statements(&PACKAGED_CATALOGS).expect("the packaged catalogs open");
    let fixture = PostgresFixture::provision_with_statements(url.as_ref(), statements)
        .resolve()
        .await
        .expect("the test database is provisioned");
    let manager = Manager::new(
        fixture
            .options
            .url
            .expose_secret()
            .parse()
            .expect("the fixture URL parses"),
        NoTls,
    );
    let pool = Pool::builder(manager)
        .max_size(1)
        .build()
        .expect("the pool builds");
    (fixture, pool)
}

#[tokio::test]
async fn an_empty_snapshot_commits_nothing() {
    let database = FakeDatabase::default();
    let committed = commit_gateway_requests_to_db(&database, &GatewayRequestSnapshot::new())
        .await
        .expect("the empty commit succeeds");
    assert_eq!(committed, 0);
    assert_eq!(database.calls(), 0);
}

#[tokio::test]
async fn a_snapshot_commits_one_statement_with_every_row() {
    let database = FakeDatabase::default();
    let rows = snapshot(&[
        ("2026-01-05", "api", "/v1/chat", 2, 1),
        ("2026-01-06", "api", "/v1/chat", 3, 0),
    ]);
    let committed = commit_gateway_requests_to_db(&database, &rows)
        .await
        .expect("the commit succeeds");
    assert_eq!(committed, 2);
    assert_eq!(database.calls(), 1);
    assert_eq!(database.last_parameters(), 10);
    let sql = database.last_sql().expect("the committed statement");
    assert!(sql.contains("ON CONFLICT (\"date\", \"category\", \"route\")"));
    assert!(sql.contains("$10::bigint"));
}

#[tokio::test]
async fn a_commit_logs_the_number_of_aggregated_rows() {
    let (logs, _guard) = captured_logs();
    let database = FakeDatabase::default();
    let rows = snapshot(&[
        ("2026-01-05", "api", "/v1/chat", 2, 1),
        ("2026-01-06", "api", "/v1/chat", 3, 0),
    ]);
    commit_gateway_requests_to_db(&database, &rows)
        .await
        .expect("the commit succeeds");
    assert!(
        logs.text()
            .contains("committed 2 aggregated rows in one statement")
    );
}

#[tokio::test]
async fn the_upsert_inserts_new_rows_and_increments_existing_ones() {
    let (fixture, pool) = database().await;
    let database = PostgresGatewayRequests {
        client: pool.get().await.expect("a pooled client"),
    };
    let rows = snapshot(&[
        ("2026-01-01", "api", "/v1/chat", 2, 1),
        ("2026-01-02", "api", "/v1/chat", 3, 0),
    ]);
    assert_eq!(
        commit_gateway_requests_to_db(&database, &rows)
            .await
            .expect("the first commit succeeds"),
        2
    );
    assert_eq!(
        commit_gateway_requests_to_db(&database, &rows)
            .await
            .expect("the second commit succeeds"),
        2
    );
    let stored = database
        .client
        .query(STORED_ROWS, &[])
        .await
        .expect("the stored rows");
    assert_eq!(stored.len(), 2);
    let first = stored.first().expect("the first stored row");
    assert_eq!(first.get::<_, String>("date"), "2026-01-01");
    assert_eq!(first.get::<_, String>("category"), "api");
    assert_eq!(first.get::<_, String>("route"), "/v1/chat");
    assert_eq!(first.get::<_, i64>("successful_requests"), 4);
    assert_eq!(first.get::<_, i64>("failed_requests"), 2);
    let second = stored.get(1).expect("the second stored row");
    assert_eq!(second.get::<_, String>("date"), "2026-01-02");
    assert_eq!(second.get::<_, i64>("successful_requests"), 6);
    assert_eq!(second.get::<_, i64>("failed_requests"), 0);
    pool.close();
    fixture.close_async().await.expect("the fixture closes");
}

#[tokio::test]
async fn the_upsert_stamps_updated_at_from_the_database_clock() {
    let (fixture, pool) = database().await;
    let database = PostgresGatewayRequests {
        client: pool.get().await.expect("a pooled client"),
    };
    let rows = snapshot(&[("2026-01-01", "api", "/v1/chat", 1, 0)]);
    commit_gateway_requests_to_db(&database, &rows)
        .await
        .expect("the commit succeeds");
    let stamped = database
        .client
        .query_one(
            r#"SELECT "updated_at" IS NOT NULL AS stamped FROM "LiteLLM_DailyGatewayRequests" WHERE "date" = '2026-01-01'"#,
            &[],
        )
        .await
        .expect("the stored row");
    assert!(stamped.get::<_, bool>("stamped"));
    pool.close();
    fixture.close_async().await.expect("the fixture closes");
}
