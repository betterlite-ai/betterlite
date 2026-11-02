use crate::schema_agent_identity::fixtures::PINNED_MIGRATION_CATALOG;
use crate::schema_agent_identity::migrations::*;
use crate::schema_agent_identity::*;
use crate::schema_baseline::{BASELINE_MIGRATION, MigrationFile};
use crate::schema_budget_windows::MIGRATION_CATALOG as BUDGET_WINDOWS_CATALOG;
use crate::schema_daily_usage::MIGRATION_CATALOG as DAILY_USAGE_CATALOG;
use crate::schema_identity_history::MIGRATION_CATALOG as IDENTITY_HISTORY_CATALOG;
use crate::schema_managed_resources::MIGRATION_CATALOG as MANAGED_RESOURCES_CATALOG;
use crate::schema_policy_resources::MIGRATION_CATALOG as POLICY_RESOURCES_CATALOG;
use crate::schema_routing_state::MIGRATION_CATALOG as ROUTING_STATE_CATALOG;
use betterlite_postgres_fixture::connection::{PostgresFixture, RandomDatabaseName};
use betterlite_postgres_fixture::error::FixtureError;
use betterlite_postgres_fixture::seed::FixtureSeed;
use clap::Parser;
use futures_util::future::join_all;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use std::future::Future;
use std::str;
use std::time::Duration;
use tokio::task::{JoinError, JoinSet};
use tokio::time::error::Elapsed;
use tokio::time::timeout;
use tokio_postgres::error::SqlState;
use tokio_postgres::types::ToSql;
use tokio_postgres::{CancelToken, Client, Row};

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

fn postgres_test_url() -> Option<SecretString> {
    Environment::parse_from(["schema-agent-identity"])
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

async fn scalar_text(client: &Client, sql: &str, params: &[&(dyn ToSql + Sync)]) -> String {
    client.query_one(sql, params).await.unwrap().get(0)
}

async fn scalar_bool(client: &Client, sql: &str, params: &[&(dyn ToSql + Sync)]) -> bool {
    client.query_one(sql, params).await.unwrap().get(0)
}

async fn scalar_opt_text(
    client: &Client,
    sql: &str,
    params: &[&(dyn ToSql + Sync)],
) -> Option<String> {
    client.query_one(sql, params).await.unwrap().get(0)
}

async fn relation_oid(client: &Client, name: &str) -> String {
    scalar_text(
        client,
        "SELECT oid::text FROM pg_class WHERE relname = $1",
        &[&name],
    )
    .await
}

async fn relation_exists(client: &Client, name: &str) -> bool {
    scalar_bool(
        client,
        "SELECT EXISTS (SELECT 1 FROM pg_class WHERE relname = $1)",
        &[&name],
    )
    .await
}

struct ColumnInfo {
    data_type: String,
    is_nullable: bool,
    default: Option<String>,
    udt_name: String,
    datetime_precision: Option<i32>,
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
        udt_name: row.get(5),
        datetime_precision: row.get(6),
    }
}

async fn columns_in(client: &Client, table: &str) -> Vec<Row> {
    client
        .query(
            "SELECT ordinal_position, column_name, data_type, is_nullable, column_default, \
             udt_name, datetime_precision \
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
    let rows = client.query(sql, params).await.unwrap();
    let mut values = Vec::with_capacity(rows.len());
    for row in &rows {
        values.push(row.get(0));
    }
    values
}

type ColumnSpec = (
    &'static str,
    &'static str,
    bool,
    Option<&'static str>,
    &'static str,
    Option<i32>,
);

async fn assert_column(client: &Client, table: &str, spec: ColumnSpec) {
    let (name, data_type, nullable, default, udt_name, datetime_precision) = spec;
    let info = column(client, table, name).await;
    assert_eq!(info.data_type, data_type, "{table}.{name} type");
    assert_eq!(info.is_nullable, nullable, "{table}.{name} nullable");
    assert_eq!(info.default.as_deref(), default, "{table}.{name} default");
    assert_eq!(info.udt_name, udt_name, "{table}.{name} udt_name");
    assert_eq!(
        info.datetime_precision, datetime_precision,
        "{table}.{name} datetime_precision"
    );
}

async fn assert_columns(client: &Client, table: &str, specs: &[ColumnSpec]) {
    for spec in specs {
        assert_column(client, table, *spec).await;
    }
}

fn bool_true(name: &'static str) -> ColumnSpec {
    (name, "boolean", false, Some("true"), "bool", None)
}

fn bool_false(name: &'static str) -> ColumnSpec {
    (name, "boolean", false, Some("false"), "bool", None)
}

fn text_default(name: &'static str, default: &'static str) -> ColumnSpec {
    (name, "text", false, Some(default), "text", None)
}

fn nullable_text(name: &'static str) -> ColumnSpec {
    (name, "text", true, None, "text", None)
}

fn nullable_text_array(name: &'static str, default: &'static str) -> ColumnSpec {
    (name, "ARRAY", true, Some(default), "_text", None)
}

fn nullable_jsonb(name: &'static str) -> ColumnSpec {
    (name, "jsonb", true, None, "jsonb", None)
}

fn nullable_empty_jsonb(name: &'static str) -> ColumnSpec {
    (name, "jsonb", true, Some("'{}'::jsonb"), "jsonb", None)
}

fn int4_zero(name: &'static str) -> ColumnSpec {
    (name, "integer", false, Some("0"), "int4", None)
}

fn int8_zero(name: &'static str) -> ColumnSpec {
    (name, "bigint", false, Some("0"), "int8", None)
}

fn float8_zero(name: &'static str) -> ColumnSpec {
    (name, "double precision", false, Some("0.0"), "float8", None)
}

fn timestamp3_now_default(name: &'static str) -> ColumnSpec {
    (
        name,
        "timestamp without time zone",
        false,
        Some("CURRENT_TIMESTAMP"),
        "timestamp",
        Some(3_i32),
    )
}

fn timestamp3_required(name: &'static str) -> ColumnSpec {
    (
        name,
        "timestamp without time zone",
        false,
        None,
        "timestamp",
        Some(3_i32),
    )
}

async fn assert_exec_params_err_state(
    client: &Client,
    sql: &str,
    params: &[&(dyn ToSql + Sync)],
    code: SqlState,
) {
    let result = exec_params_err(client, sql, params).await;
    assert_eq!(sql_state(&result), code.code());
}

async fn catalog_snapshot(client: &Client) -> Vec<String> {
    let mut snapshot = string_rows(
        client,
        "SELECT table_name || ':' || ordinal_position || ':' || column_name || ':' \
         || data_type || ':' || udt_name || ':' || coalesce(datetime_precision::text, '') \
         || ':' || is_nullable || ':' || coalesce(column_default, '') \
         FROM information_schema.columns \
         WHERE table_schema = 'public' ORDER BY table_name, ordinal_position",
        &[],
    )
    .await;
    let mut indexes = string_rows(
        client,
        "SELECT indexname || ':' || indexdef FROM pg_indexes \
         WHERE schemaname = 'public' ORDER BY indexname",
        &[],
    )
    .await;
    snapshot.append(&mut indexes);
    let mut constraints = string_rows(
        client,
        "SELECT conname || ':' || pg_get_constraintdef(oid) FROM pg_constraint \
         WHERE connamespace = 'public'::regnamespace ORDER BY conname",
        &[],
    )
    .await;
    snapshot.append(&mut constraints);
    snapshot
}

async fn assert_rule(client: &Client, conname: &str, update_rule: char, delete_rule: char) {
    let row = query_one_row(
        client,
        "SELECT confupdtype, confdeltype FROM pg_constraint WHERE conname = $1",
        &[&conname],
    )
    .await;
    let update: i8 = row.get(0);
    let delete: i8 = row.get(1);
    assert_eq!(char::from(u8::try_from(update).unwrap()), update_rule);
    assert_eq!(char::from(u8::try_from(delete).unwrap()), delete_rule);
}

fn sql_state(error: &tokio_postgres::Error) -> &str {
    error.code().map_or("", SqlState::code)
}

async fn apply_baseline(client: &Client) {
    batch_execute(client, str::from_utf8(BASELINE_MIGRATION.bytes).unwrap()).await;
    for catalog in [
        DAILY_USAGE_CATALOG,
        MANAGED_RESOURCES_CATALOG,
        IDENTITY_HISTORY_CATALOG,
        POLICY_RESOURCES_CATALOG,
        ROUTING_STATE_CATALOG,
        BUDGET_WINDOWS_CATALOG,
    ] {
        for file in catalog.files {
            batch_execute(client, str::from_utf8(file.bytes).unwrap()).await;
        }
    }
}

async fn apply_migrations_through(client: &Client, directory: &str) {
    apply_baseline(client).await;
    for file in MIGRATION_CATALOG.files {
        batch_execute(client, str::from_utf8(file.bytes).unwrap()).await;
        if file.directory == directory {
            return;
        }
    }
    panic!("no migration named {directory} in MIGRATION_CATALOG");
}

async fn apply_migrations_all(client: &Client) {
    let last = MIGRATION_CATALOG.files.last().unwrap();
    apply_migrations_through(client, last.directory).await;
}

async fn apply_pinned_migrations_all(client: &Client) {
    apply_baseline(client).await;
    for file in PINNED_MIGRATION_CATALOG.files {
        batch_execute(client, str::from_utf8(file.bytes).unwrap()).await;
    }
}

fn migration_sql_of(directory: &str) -> &'static str {
    let file = MIGRATION_CATALOG
        .files
        .iter()
        .find(|file| file.directory == directory)
        .unwrap_or_else(|| panic!("no migration named {directory} in MIGRATION_CATALOG"));
    str::from_utf8(file.bytes).unwrap()
}
