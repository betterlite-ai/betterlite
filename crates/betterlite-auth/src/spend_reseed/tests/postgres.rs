use betterlite_db::budget_permission_rows::Timestamp;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use chrono::NaiveDateTime;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;
use tokio_postgres::types::ToSql;

use super::support::{decimal, utc};
use crate::spend_reseed::rows::{SpendCounterRows as _, SpendRowLookup, WindowEntity};
use crate::spend_reseed::sql::SqlSpendCounterRows;
use crate::spend_reseed::values::{aggregate_amount, stored_amount};

const SCHEMA: &str = r#"
CREATE TABLE "LiteLLM_VerificationToken" (
    token TEXT PRIMARY KEY,
    spend DOUBLE PRECISION,
    max_budget DOUBLE PRECISION,
    blocked BOOLEAN NOT NULL DEFAULT false
);
CREATE TABLE "LiteLLM_TeamMembership" (
    user_id TEXT NOT NULL,
    team_id TEXT NOT NULL,
    spend DOUBLE PRECISION,
    budget_id TEXT,
    PRIMARY KEY (user_id, team_id)
);
CREATE TABLE "LiteLLM_TeamTable" (
    team_id TEXT PRIMARY KEY,
    spend DOUBLE PRECISION,
    max_budget DOUBLE PRECISION
);
CREATE TABLE "LiteLLM_UserTable" (
    user_id TEXT PRIMARY KEY,
    spend DOUBLE PRECISION,
    user_email TEXT
);
CREATE TABLE "LiteLLM_OrganizationTable" (
    organization_id TEXT PRIMARY KEY,
    spend DOUBLE PRECISION
);
CREATE TABLE "LiteLLM_ProjectTable" (
    project_id TEXT PRIMARY KEY,
    spend DOUBLE PRECISION
);
CREATE TABLE "LiteLLM_EndUserTable" (
    user_id TEXT PRIMARY KEY,
    spend DOUBLE PRECISION
);
CREATE TABLE "LiteLLM_BudgetWindowSpend" (
    entity_type TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    window_duration TEXT NOT NULL,
    window_start TIMESTAMP(3) NOT NULL,
    spend DOUBLE PRECISION,
    PRIMARY KEY (entity_type, entity_id, window_duration)
);
CREATE TABLE "LiteLLM_SpendLogs" (
    request_id TEXT,
    api_key TEXT,
    team_id TEXT,
    spend DOUBLE PRECISION,
    "startTime" TIMESTAMP(3) NOT NULL
);
"#;

const LOG_ROW: &str = r#"INSERT INTO "LiteLLM_SpendLogs" (api_key, team_id, spend, "startTime") VALUES ($1, $2, $3, $4)"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn database() -> (PostgresFixture, Pool) {
    let url = Environment::parse_from(["spend-reseed-tests"])
        .url
        .map(SecretString::from);
    let fixture = PostgresFixture::provision_with_statements(url.as_ref(), vec![SCHEMA.to_owned()])
        .resolve()
        .await
        .expect("the database fixture");
    let manager = Manager::new(
        fixture
            .options
            .url
            .expose_secret()
            .parse()
            .expect("the database url"),
        NoTls,
    );
    let pool = Pool::builder(manager)
        .max_size(2)
        .build()
        .expect("the database pool");
    (fixture, pool)
}

fn rows_of(pool: &Pool) -> SqlSpendCounterRows {
    SqlSpendCounterRows::from(PostgresStore {
        pool: Some(pool.clone()),
    })
}

async fn execute(pool: &Pool, statement: &str, parameters: &[&(dyn ToSql + Sync)]) {
    let client = pool.get().await.expect("the database connection");
    client
        .execute(statement, parameters)
        .await
        .expect("the fixture row");
}

fn at(day: u32, hour: u32) -> NaiveDateTime {
    let text = format!("2026-11-{day:02}T{hour:02}:00:00");
    NaiveDateTime::parse_from_str(&text, "%Y-%m-%dT%H:%M:%S").expect("a fixture moment")
}

async fn insert_spend(pool: &Pool, table: &str, key_column: &str, identifier: &str, spend: f64) {
    let statement = format!(r#"INSERT INTO "{table}" ({key_column}, spend) VALUES ($1, $2)"#);
    execute(pool, &statement, &[&identifier, &spend]).await;
}

async fn insert_window(pool: &Pool, entity: &str, identifier: &str, duration: &str, spend: f64) {
    let statement = format!(
        r#"INSERT INTO "LiteLLM_BudgetWindowSpend" (entity_type, entity_id, window_duration, window_start, spend) VALUES ($1, $2, $3, $4, $5)"#
    );
    execute(
        pool,
        &statement,
        &[&entity, &identifier, &duration, &at(18, 0), &spend],
    )
    .await;
}

async fn insert_log(pool: &Pool, api_key: &str, team_id: &str, spend: f64, moment: NaiveDateTime) {
    execute(pool, LOG_ROW, &[&api_key, &team_id, &spend, &moment]).await;
}

async fn seed_primary_rows(pool: &Pool) {
    let rows = [
        ("LiteLLM_VerificationToken", "token", "hashed-key", 2.5),
        ("LiteLLM_TeamTable", "team_id", "t1", 3.25),
        ("LiteLLM_UserTable", "user_id", "u1", 4.5),
        ("LiteLLM_OrganizationTable", "organization_id", "org1", 5.25),
        ("LiteLLM_ProjectTable", "project_id", "proj1", 6.75),
        ("LiteLLM_EndUserTable", "user_id", "eu1", 7.25),
    ];
    for (table, column, identifier, spend) in rows {
        insert_spend(pool, table, column, identifier, spend).await;
    }
    execute(
        pool,
        r#"INSERT INTO "LiteLLM_VerificationToken" (token, spend) VALUES ($1, NULL)"#,
        &[&"null-spend"],
    )
    .await;
    execute(
        pool,
        r#"INSERT INTO "LiteLLM_TeamMembership" (user_id, team_id, spend) VALUES ($1, $2, $3)"#,
        &[&"u1", &"t1", &1.75_f64],
    )
    .await;
}

async fn seed_window_rows(pool: &Pool) {
    insert_window(pool, "key", "hashed-key", "1h", 6.5).await;
    insert_window(pool, "team", "t1", "1d", 2.5).await;
    insert_log(pool, "hashed-key", "none", 10.0, at(17, 23)).await;
    insert_log(pool, "hashed-key", "none", 0.25, at(18, 0)).await;
    insert_log(pool, "hashed-key", "none", 1.5, at(18, 1)).await;
    insert_log(pool, "other-key", "none", 50.0, at(18, 1)).await;
    insert_log(pool, "none", "t1", 2.0, at(18, 2)).await;
    insert_log(pool, "none", "t1", 4.0, at(18, 3)).await;
}

async fn close(fixture: PostgresFixture, pool: Pool) {
    pool.close();
    fixture.close_async().await.expect("the fixture cleanup");
}

#[rstest]
#[case::key(SpendRowLookup::Key("hashed-key"), "2.5")]
#[case::team_member(
    SpendRowLookup::TeamMember {
        user_id: "u1",
        team_id: "t1",
    },
    "1.75"
)]
#[case::team(SpendRowLookup::Team("t1"), "3.25")]
#[case::user(SpendRowLookup::User("u1"), "4.5")]
#[case::organization(SpendRowLookup::Organization("org1"), "5.25")]
#[case::project(SpendRowLookup::Project("proj1"), "6.75")]
#[case::end_user(SpendRowLookup::EndUser("eu1"), "7.25")]
#[tokio::test]
async fn primary_rows_from_the_database(
    #[case] lookup: SpendRowLookup<'static>,
    #[case] expected: &str,
) {
    let (fixture, pool) = database().await;
    seed_primary_rows(&pool).await;
    let rows = rows_of(&pool);
    let stored = rows.spend_row(lookup).await.expect("the spend row");
    let found = stored.expect("a stored row");
    assert_eq!(stored_amount(found.spend.as_ref()), Ok(decimal(expected)));
    let absent = rows.spend_row(SpendRowLookup::Key("other")).await;
    assert_eq!(absent.expect("the spend row"), None);
    let null_spend = rows.spend_row(SpendRowLookup::Key("null-spend")).await;
    let null_row = null_spend.expect("the spend row").expect("a stored row");
    assert_eq!(stored_amount(null_row.spend.as_ref()), Ok(decimal("0")));
    let member = rows.spend_row(SpendRowLookup::TeamMember {
        user_id: "u2",
        team_id: "t1",
    });
    assert_eq!(member.await.expect("the spend row"), None);
    close(fixture, pool).await;
}

#[tokio::test]
async fn window_rows_from_the_database() {
    let (fixture, pool) = database().await;
    seed_window_rows(&pool).await;
    let rows = rows_of(&pool);
    let maintained = rows.window_row(WindowEntity::Key, "hashed-key", "1h").await;
    let key_row = maintained
        .expect("the window row")
        .expect("a stored window");
    assert_eq!(key_row.window_start, Timestamp::Naive(at(18, 0)));
    assert_eq!(stored_amount(key_row.spend.as_ref()), Ok(decimal("6.5")));
    let team = rows.window_row(WindowEntity::Team, "t1", "1d").await;
    let team_row = team.expect("the window row").expect("a stored window");
    assert_eq!(stored_amount(team_row.spend.as_ref()), Ok(decimal("2.5")));
    let duration_miss = rows.window_row(WindowEntity::Key, "hashed-key", "1d").await;
    assert_eq!(duration_miss.expect("the window row"), None);
    let entity_miss = rows
        .window_row(WindowEntity::Team, "hashed-key", "1h")
        .await;
    assert_eq!(entity_miss.expect("the window row"), None);
    close(fixture, pool).await;
}

#[tokio::test]
async fn aggregate_queries_bound_and_entity_filter() {
    let (fixture, pool) = database().await;
    seed_window_rows(&pool).await;
    let rows = rows_of(&pool);
    let midnight = utc("2026-11-18T00:00:00Z");
    let boundary = rows
        .window_log_rows(WindowEntity::Key, "hashed-key", midnight)
        .await;
    let boundary_rows = boundary.expect("the key logs");
    assert_eq!(boundary_rows.len(), 1);
    let boundary_sum = boundary_rows.first().expect("an aggregate row");
    assert_eq!(aggregate_amount(boundary_sum), Ok(decimal("1.75")));
    let later = rows
        .window_log_rows(WindowEntity::Key, "hashed-key", utc("2026-11-18T00:00:01Z"))
        .await;
    let later_rows = later.expect("the key logs");
    let later_sum = later_rows.first().expect("an aggregate row");
    assert_eq!(aggregate_amount(later_sum), Ok(decimal("1.5")));
    let teams = rows
        .window_log_rows(WindowEntity::Team, "t1", midnight)
        .await;
    let team_rows = teams.expect("the team logs");
    let team_sum = team_rows.first().expect("an aggregate row");
    assert_eq!(aggregate_amount(team_sum), Ok(decimal("6")));
    let without_logs = rows
        .window_log_rows(WindowEntity::Team, "t3", midnight)
        .await;
    assert!(without_logs.expect("the team logs").is_empty());
    close(fixture, pool).await;
}
