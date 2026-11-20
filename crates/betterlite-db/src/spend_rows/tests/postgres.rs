use std::sync::Arc;

use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;
use tokio_postgres::types::ToSql;

use crate::postgres_store::session::PostgresStore;
use crate::spend_rows::reader::SpendCounterReader;
use crate::spend_rows::rows::{SpendCounterRows as _, SpendRow, SpendRowLookup};
use crate::spend_rows::sql::SqlSpendCounterRows;
use crate::spend_rows::tests::support::{FixedDurations, decimal, services};

const TABLES: &str = r#"
CREATE TABLE "LiteLLM_VerificationToken" (token TEXT PRIMARY KEY, spend DOUBLE PRECISION);
CREATE TABLE "LiteLLM_TeamMembership" (user_id TEXT NOT NULL, team_id TEXT NOT NULL, spend DOUBLE PRECISION, PRIMARY KEY (user_id, team_id));
CREATE TABLE "LiteLLM_TeamTable" (team_id TEXT PRIMARY KEY, spend DOUBLE PRECISION);
CREATE TABLE "LiteLLM_UserTable" (user_id TEXT PRIMARY KEY, spend DOUBLE PRECISION);
CREATE TABLE "LiteLLM_OrganizationTable" (organization_id TEXT PRIMARY KEY, spend DOUBLE PRECISION);
CREATE TABLE "LiteLLM_ProjectTable" (project_id TEXT PRIMARY KEY, spend DOUBLE PRECISION);
"#;

const END_USER_TABLE: &str =
    r#"CREATE TABLE "LiteLLM_EndUserTable" (user_id TEXT PRIMARY KEY, spend DOUBLE PRECISION);"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn database(with_end_user: bool) -> (PostgresFixture, Pool) {
    let url = Environment::parse_from(["spend-rows-tests"])
        .url
        .map(SecretString::from);
    let statements = if with_end_user {
        vec![format!("{TABLES}\n{END_USER_TABLE}")]
    } else {
        vec![TABLES.to_owned()]
    };
    let fixture = PostgresFixture::provision_with_statements(url.as_ref(), statements)
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

async fn seed_key_rows(pool: &Pool) {
    execute(
        pool,
        r#"INSERT INTO "LiteLLM_VerificationToken" (token, spend) VALUES ($1, $2)"#,
        &[&"hashed-key", &2.5_f64],
    )
    .await;
    execute(
        pool,
        r#"INSERT INTO "LiteLLM_VerificationToken" (token, spend) VALUES ($1, NULL)"#,
        &[&"null-spend"],
    )
    .await;
}

async fn seed_entity_rows(pool: &Pool) {
    execute(
        pool,
        r#"INSERT INTO "LiteLLM_TeamMembership" (user_id, team_id, spend) VALUES ($1, $2, $3)"#,
        &[&"u1", &"t1", &1.75_f64],
    )
    .await;
    execute(
        pool,
        r#"INSERT INTO "LiteLLM_TeamTable" (team_id, spend) VALUES ($1, $2)"#,
        &[&"t1", &3.25_f64],
    )
    .await;
    execute(
        pool,
        r#"INSERT INTO "LiteLLM_UserTable" (user_id, spend) VALUES ($1, $2)"#,
        &[&"u1", &4.5_f64],
    )
    .await;
    execute(
        pool,
        r#"INSERT INTO "LiteLLM_OrganizationTable" (organization_id, spend) VALUES ($1, $2)"#,
        &[&"org1", &5.25_f64],
    )
    .await;
    execute(
        pool,
        r#"INSERT INTO "LiteLLM_ProjectTable" (project_id, spend) VALUES ($1, $2)"#,
        &[&"proj1", &6.75_f64],
    )
    .await;
    execute(
        pool,
        r#"INSERT INTO "LiteLLM_EndUserTable" (user_id, spend) VALUES ($1, $2)"#,
        &[&"eu1", &7.25_f64],
    )
    .await;
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
async fn stored_rows_carry_each_entity_spend(
    #[case] lookup: SpendRowLookup<'static>,
    #[case] expected: &str,
) {
    let (fixture, pool) = database(true).await;
    seed_key_rows(&pool).await;
    seed_entity_rows(&pool).await;
    let rows = rows_of(&pool);
    let found = rows
        .spend_row(lookup)
        .await
        .expect("the spend row")
        .expect("a stored row");
    assert_eq!(
        found,
        SpendRow {
            spend: Some(decimal(expected))
        }
    );
    let absent = rows.spend_row(SpendRowLookup::Key("absent")).await;
    assert_eq!(absent.expect("the spend row"), None);
    let null_spend = rows.spend_row(SpendRowLookup::Key("null-spend")).await;
    assert_eq!(
        null_spend.expect("the spend row"),
        Some(SpendRow { spend: None })
    );
    close(fixture, pool).await;
}

#[tokio::test]
async fn the_reader_reads_stored_spend_through_the_gate() {
    let (fixture, pool) = database(true).await;
    seed_key_rows(&pool).await;
    seed_entity_rows(&pool).await;
    let services = services(4);
    let reader = SpendCounterReader {
        rows: Some(Arc::new(rows_of(&pool))),
        durations: Arc::new(FixedDurations),
        tasks: services.tasks,
        gate: services.gate,
        stall_tracker: services.stall_tracker,
        deadline_seconds: 5.0,
    };
    assert_eq!(
        reader.from_db("spend:key:hashed-key").await,
        Some(decimal("2.5"))
    );
    assert_eq!(
        reader.from_db("spend:team_member:u1:t1").await,
        Some(decimal("1.75"))
    );
    assert_eq!(reader.from_db("spend:key:absent").await, None);
    assert_eq!(reader.from_db("spend:key:hashed-key:window:1h").await, None);
    assert_eq!(
        reader.end_user_from_db("spend:end_user:eu1").await,
        Some(decimal("7.25"))
    );
    assert_eq!(reader.from_db("spend:end_user:eu1").await, None);
    close(fixture, pool).await;
}

#[tokio::test]
async fn a_failed_floor_read_returns_none() {
    let (fixture, pool) = database(false).await;
    let services = services(4);
    let reader = SpendCounterReader {
        rows: Some(Arc::new(rows_of(&pool))),
        durations: Arc::new(FixedDurations),
        tasks: services.tasks,
        gate: services.gate,
        stall_tracker: services.stall_tracker,
        deadline_seconds: 5.0,
    };
    assert_eq!(reader.end_user_from_db("spend:end_user:eu1").await, None);
    close(fixture, pool).await;
}
