use crate::schema_baseline::{BASELINE_MIGRATION, MigrationFile};
use crate::schema_daily_usage::MIGRATION_CATALOG;
use betterlite_postgres_fixture::connection::{PostgresFixture, RandomDatabaseName};
use betterlite_postgres_fixture::error::FixtureError;
use betterlite_postgres_fixture::seed::FixtureSeed;
use clap::Parser;
use futures_util::future::join_all;
use secrecy::{ExposeSecret as _, SecretString};
use std::future::Future;
use std::str;
use std::time::Duration;
use tokio::task::{JoinError, JoinSet};
use tokio::time::error::Elapsed;
use tokio::time::timeout;
use tokio_postgres::types::ToSql;
use tokio_postgres::{CancelToken, Client, Row};

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

fn postgres_test_url() -> Option<SecretString> {
    Environment::parse_from(["schema-daily-usage"])
        .url
        .map(SecretString::from)
}

type ConnectionTasks = JoinSet<Result<(), tokio_postgres::Error>>;

const RESET_PUBLIC_SCHEMA_SQL: &str = "DROP SCHEMA public CASCADE; CREATE SCHEMA public;";
const SETUP_TIMEOUT: Duration = Duration::from_secs(30);
const BODY_TIMEOUT: Duration = Duration::from_secs(30);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, thiserror::Error)]
enum HarnessError {
    #[error("fixture operation timed out: {0}")]
    Timeout(#[from] Elapsed),
    #[error("fixture database operation failed: {0}")]
    Database(#[from] tokio_postgres::Error),
    #[error("fixture lifecycle failed: {0}")]
    Fixture(#[from] FixtureError),
    #[error("test body failed: {0}")]
    Body(#[from] JoinError),
}

async fn provision() -> Result<PostgresFixture, HarnessError> {
    let url = postgres_test_url();
    let mut generator = RandomDatabaseName;
    let mut task = PostgresFixture::provision_with_name_generator(
        url.as_ref(),
        &FixtureSeed::default(),
        &mut generator,
    );
    let result = timeout(SETUP_TIMEOUT, task.resolve())
        .await
        .map_err(HarnessError::Timeout)
        .and_then(|result| result.map_err(HarnessError::Fixture));
    if result.is_err() {
        timeout(SHUTDOWN_TIMEOUT, task.close()).await??;
    }
    result
}

async fn provision_all<const N: usize>() -> [PostgresFixture; N] {
    let mut fixtures = Vec::with_capacity(N);
    for _ in 0..N {
        match Box::pin(provision()).await {
            Ok(fixture) => fixtures.push(fixture),
            Err(error) => {
                close_fixtures(fixtures).await;
                panic!("failed to provision fixture: {error}");
            }
        }
    }
    fixtures.try_into().unwrap()
}

async fn initialize_schema(
    fixture: &PostgresFixture,
    sql: &str,
    duration: Duration,
) -> Result<(Client, ConnectionTasks), HarnessError> {
    let (client, connection) = timeout(
        duration,
        tokio_postgres::connect(fixture.options.url.expose_secret(), tokio_postgres::NoTls),
    )
    .await??;
    let mut tasks = JoinSet::new();
    tasks.spawn(connection);
    let result = timeout(duration, client.batch_execute(sql))
        .await
        .map_err(HarnessError::Timeout)
        .and_then(|result| result.map_err(HarnessError::Database));
    match result {
        Ok(()) => Ok((client, tasks)),
        Err(error) => {
            let cancelled = cancel_queries(&[client.cancel_token()]).await;
            drop(client);
            let drained = drain_connection(tasks).await;
            cancelled?;
            drained?;
            Err(error)
        }
    }
}

async fn initialize_all(
    fixtures: &[PostgresFixture],
    connections: &mut Vec<ConnectionTasks>,
) -> Result<Vec<Client>, HarnessError> {
    let mut clients = Vec::with_capacity(fixtures.len());
    for fixture in fixtures {
        let (client, tasks) =
            initialize_schema(fixture, RESET_PUBLIC_SCHEMA_SQL, SETUP_TIMEOUT).await?;
        clients.push(client);
        connections.push(tasks);
    }
    Ok(clients)
}

async fn cancel_queries(tokens: &[CancelToken]) -> Result<(), HarnessError> {
    let results = join_all(
        tokens
            .iter()
            .map(|token| timeout(SHUTDOWN_TIMEOUT, token.cancel_query(tokio_postgres::NoTls))),
    )
    .await;
    for result in results {
        result??;
    }
    Ok(())
}

async fn drain_connection(mut tasks: ConnectionTasks) -> Result<(), HarnessError> {
    let result = timeout(SHUTDOWN_TIMEOUT, async {
        while let Some(outcome) = tasks.join_next().await {
            outcome??;
        }
        Ok(())
    })
    .await;
    timeout(SHUTDOWN_TIMEOUT, tasks.shutdown()).await?;
    result?
}

async fn close_fixtures(fixtures: Vec<PostgresFixture>) {
    let outcomes = join_all(
        fixtures
            .into_iter()
            .map(|fixture| async move { timeout(SHUTDOWN_TIMEOUT, fixture.close_async()).await }),
    )
    .await;
    for outcome in outcomes {
        outcome.unwrap().unwrap();
    }
}

async fn run_body<Fut>(body: Fut, duration: Duration) -> Result<(), HarnessError>
where
    Fut: Future<Output = ()> + Send + 'static,
{
    let mut bodies = JoinSet::new();
    bodies.spawn(body);
    let result = timeout(duration, bodies.join_next()).await;
    timeout(SHUTDOWN_TIMEOUT, bodies.shutdown()).await?;
    result?.unwrap().map_err(HarnessError::Body)
}

async fn harness_on<const N: usize, F, Fut>(
    fixtures: [PostgresFixture; N],
    body: F,
    duration: Duration,
) -> Result<(), HarnessError>
where
    F: FnOnce([Client; N]) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let mut connections = Vec::with_capacity(N);
    let mut tokens = Vec::with_capacity(N);
    let outcome = match initialize_all(&fixtures, &mut connections).await {
        Ok(clients) => {
            tokens.extend(clients.iter().map(Client::cancel_token));
            Box::pin(run_body(
                async move {
                    body(clients.try_into().unwrap()).await;
                },
                duration,
            ))
            .await
        }
        Err(error) => Err(error),
    };
    let cancelled = if outcome.is_err() {
        cancel_queries(&tokens).await
    } else {
        Ok(())
    };
    let drained = join_all(connections.into_iter().map(drain_connection)).await;
    close_fixtures(Vec::from(fixtures)).await;
    cancelled?;
    for result in drained {
        result?;
    }
    outcome
}

async fn harness<const N: usize, F, Fut>(body: F)
where
    F: FnOnce([Client; N]) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let fixtures = Box::pin(provision_all()).await;
    Box::pin(harness_on(fixtures, body, BODY_TIMEOUT))
        .await
        .unwrap();
}

async fn batch_execute(client: &Client, sql: &str) {
    client.batch_execute(sql).await.unwrap();
}

async fn batch_execute_err(client: &Client, sql: &str) -> tokio_postgres::Error {
    client.batch_execute(sql).await.unwrap_err()
}

async fn exec_params(client: &Client, sql: &str, params: &[&(dyn ToSql + Sync)]) {
    client.execute(sql, params).await.unwrap();
}

async fn exec_params_err(
    client: &Client,
    sql: &str,
    params: &[&(dyn ToSql + Sync)],
) -> tokio_postgres::Error {
    client.execute(sql, params).await.unwrap_err()
}

async fn scalar_text(client: &Client, sql: &str) -> String {
    client.query_one(sql, &[]).await.unwrap().get(0)
}

async fn scalar_i64(client: &Client, sql: &str, params: &[&(dyn ToSql + Sync)]) -> i64 {
    client.query_one(sql, params).await.unwrap().get(0)
}

async fn create_decoy_constraint(client: &Client, conname: &str) {
    client
        .batch_execute(&format!(
            r#"CREATE SCHEMA "other"; CREATE TABLE "other"."Decoy" ("decoy_id" TEXT NOT NULL CONSTRAINT "{conname}" CHECK ("decoy_id" IS NOT NULL));"#
        ))
        .await
        .unwrap();
}

async fn membership_row(client: &Client, user_id: &str, team_id: &str) -> Option<Row> {
    let sql =
        r#"SELECT budget_id FROM "LiteLLM_TeamMembership" WHERE user_id = $1 AND team_id = $2"#;
    query_opt_row(client, sql, &[&user_id, &team_id]).await
}

async fn membership_budget_id(client: &Client, user_id: &str, team_id: &str) -> Option<String> {
    membership_row(client, user_id, team_id)
        .await
        .unwrap()
        .get(0)
}

async fn membership_exists(client: &Client, user_id: &str, team_id: &str) -> bool {
    membership_row(client, user_id, team_id).await.is_some()
}

struct ColumnInfo {
    data_type: String,
    is_nullable: bool,
    default: Option<String>,
}

async fn column(client: &Client, table: &str, name: &str) -> ColumnInfo {
    let rows = columns_in(client, table).await;
    let row = rows
        .iter()
        .find(|row| row.get::<_, String>(1) == name)
        .unwrap();
    ColumnInfo {
        data_type: row.get(2),
        is_nullable: row.get::<_, String>(3) == "YES",
        default: row.get(4),
    }
}

async fn columns_in(client: &Client, table: &str) -> Vec<Row> {
    client
        .query(
            "SELECT ordinal_position, column_name, data_type, is_nullable, column_default \
             FROM information_schema.columns WHERE table_schema = 'public' AND table_name = $1 \
             ORDER BY ordinal_position",
            &[&table],
        )
        .await
        .unwrap()
}

async fn query_one_row(client: &Client, sql: &str, params: &[&(dyn ToSql + Sync)]) -> Row {
    client.query_one(sql, params).await.unwrap()
}

async fn query_opt_row(client: &Client, sql: &str, params: &[&(dyn ToSql + Sync)]) -> Option<Row> {
    client.query_opt(sql, params).await.unwrap()
}

async fn string_rows(client: &Client, sql: &str, params: &[&(dyn ToSql + Sync)]) -> Vec<String> {
    client
        .query(sql, params)
        .await
        .unwrap()
        .iter()
        .map(|row| row.get(0))
        .collect()
}

struct IndexFacts {
    columns: Vec<String>,
    unique: bool,
}

async fn index_row(client: &Client, index: &str) -> Option<Row> {
    query_opt_row(
        client,
        "SELECT indisunique FROM pg_index idx \
         JOIN pg_class ic ON ic.oid = idx.indexrelid WHERE ic.relname = $1",
        &[&index],
    )
    .await
}

async fn index_exists(client: &Client, index: &str) -> bool {
    index_row(client, index).await.is_some()
}

async fn index_facts(client: &Client, index: &str) -> IndexFacts {
    let unique = index_row(client, index).await.unwrap().get(0);
    let columns = string_rows(
        client,
        "SELECT a.attname FROM pg_index idx \
         JOIN pg_class ic ON ic.oid = idx.indexrelid \
         JOIN LATERAL unnest(idx.indkey) WITH ORDINALITY AS k(attnum, ord) ON true \
         JOIN pg_attribute a ON a.attrelid = idx.indrelid AND a.attnum = k.attnum \
         WHERE ic.relname = $1 ORDER BY k.ord",
        &[&index],
    )
    .await;
    IndexFacts { columns, unique }
}

async fn assert_rule(client: &Client, conname: &str, delete_rule: char) {
    let row = query_one_row(
        client,
        "SELECT confupdtype, confdeltype FROM pg_constraint WHERE conname = $1",
        &[&conname],
    )
    .await;
    let update: i8 = row.get(0);
    let delete: i8 = row.get(1);
    assert_eq!(char::from(u8::try_from(update).unwrap()), 'c');
    assert_eq!(char::from(u8::try_from(delete).unwrap()), delete_rule);
}

fn migration_sql_through(directory: &str) -> String {
    let mut sql = str::from_utf8(BASELINE_MIGRATION.bytes).unwrap().to_owned();
    for file in MIGRATION_CATALOG.files {
        sql.push_str(str::from_utf8(file.bytes).unwrap());
        if file.directory == directory {
            return sql;
        }
    }
    panic!("no migration named {directory} in MIGRATION_CATALOG");
}

fn migration_sql_all() -> String {
    let last = MIGRATION_CATALOG.files.last().unwrap();
    migration_sql_through(last.directory)
}

fn migration_sql_between(after_exclusive: &str, through_inclusive: &str) -> String {
    let mut sql = String::new();
    let mut in_range = after_exclusive.is_empty();
    for file in MIGRATION_CATALOG.files {
        if in_range {
            sql.push_str(str::from_utf8(file.bytes).unwrap());
        }
        if !in_range && file.directory == after_exclusive {
            in_range = true;
        }
        if file.directory == through_inclusive {
            return sql;
        }
    }
    panic!("range not found: {after_exclusive}..={through_inclusive}");
}

fn migration_sql_of(directory: &str) -> &'static str {
    let file = MIGRATION_CATALOG
        .files
        .iter()
        .find(|file| file.directory == directory)
        .unwrap_or_else(|| panic!("no migration named {directory} in MIGRATION_CATALOG"));
    str::from_utf8(file.bytes).unwrap()
}
