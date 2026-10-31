use betterlite_postgres_fixture::connection::{PostgresFixture, RandomDatabaseName};
use betterlite_postgres_fixture::seed::FixtureSeed;
use clap::Parser;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use std::future::Future;
use std::str;
use std::time::Duration;
use tokio::task::JoinSet;
use tokio::time::timeout;
use tokio_postgres::Client;
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;

fn migration_sql() -> &'static str {
    str::from_utf8(BASELINE_MIGRATION.bytes).unwrap()
}

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

fn postgres_test_url() -> Option<SecretString> {
    Environment::parse_from(["schema-baseline"])
        .url
        .map(SecretString::from)
}

type ConnectionTasks = JoinSet<Result<(), tokio_postgres::Error>>;

const RESET_PUBLIC_SCHEMA_SQL: &str = "DROP SCHEMA public CASCADE; CREATE SCHEMA public;";
const SETUP_TIMEOUT: Duration = Duration::from_secs(30);
const HARNESS_SETUP_TIMEOUT: Duration = Duration::from_secs(90);
const BODY_TIMEOUT: Duration = Duration::from_secs(30);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

async fn provision() -> PostgresFixture {
    let url = postgres_test_url();
    let mut generator = RandomDatabaseName;
    let mut task = PostgresFixture::provision_with_name_generator(
        url.as_ref(),
        &FixtureSeed::default(),
        &mut generator,
    );
    match timeout(SETUP_TIMEOUT, task.resolve()).await {
        Ok(Ok(fixture)) => fixture,
        Ok(Err(error)) => {
            timeout(SHUTDOWN_TIMEOUT, task.close())
                .await
                .unwrap_or_else(|_| panic!("timed out cleaning up fixture provisioning"))
                .unwrap();
            panic!("failed to provision fixture: {error}");
        }
        Err(_) => {
            timeout(SHUTDOWN_TIMEOUT, task.close())
                .await
                .unwrap_or_else(|_| panic!("timed out cleaning up fixture provisioning"))
                .unwrap();
            panic!("timed out provisioning fixture");
        }
    }
}

async fn initialize_schema(
    fixture: &PostgresFixture,
    sql: &str,
) -> Result<(Client, ConnectionTasks), String> {
    let connected = timeout(
        SETUP_TIMEOUT,
        tokio_postgres::connect(fixture.options.url.expose_secret(), tokio_postgres::NoTls),
    )
    .await
    .map_err(|error| format!("timed out connecting to fixture: {error}"))?
    .map_err(|error| error.to_string())?;
    let (client, connection) = connected;
    let mut tasks = JoinSet::new();
    tasks.spawn(connection);
    match timeout(SETUP_TIMEOUT, client.batch_execute(sql)).await {
        Ok(Ok(())) => Ok((client, tasks)),
        Ok(Err(error)) => {
            drop(client);
            close_connection_tasks(tasks).await;
            Err(error.to_string())
        }
        Err(_) => {
            drop(client);
            close_connection_tasks(tasks).await;
            Err("timed out initializing fixture schema".to_owned())
        }
    }
}

async fn initialize_or_close(
    fixture: PostgresFixture,
    sql: &str,
) -> (PostgresFixture, Client, ConnectionTasks) {
    match initialize_schema(&fixture, sql).await {
        Ok((client, tasks)) => (fixture, client, tasks),
        Err(err) => {
            timeout(SHUTDOWN_TIMEOUT, fixture.close_async())
                .await
                .unwrap_or_else(|_| panic!("timed out closing failed fixture setup"))
                .unwrap();
            panic!("failed to initialize schema: {err}");
        }
    }
}

async fn setup() -> (PostgresFixture, Client, ConnectionTasks) {
    initialize_or_close(Box::pin(provision()).await, RESET_PUBLIC_SCHEMA_SQL).await
}

async fn close_fixture(tasks: ConnectionTasks, fixture: PostgresFixture) {
    close_connection_tasks(tasks).await;
    timeout(SHUTDOWN_TIMEOUT, fixture.close_async())
        .await
        .unwrap_or_else(|_| panic!("timed out closing fixture"))
        .unwrap();
}

async fn close_connection_tasks(mut tasks: ConnectionTasks) {
    let drained = timeout(SHUTDOWN_TIMEOUT, async {
        while tasks.join_next().await.is_some() {}
    })
    .await;
    if drained.is_err() {
        tasks.abort_all();
        let drained_after_abort = timeout(SHUTDOWN_TIMEOUT, async {
            while tasks.join_next().await.is_some() {}
        })
        .await;
        if drained_after_abort.is_err() {
            tasks.abort_all();
        }
    }
}

enum BodyFailure {
    Panicked,
    TimedOut,
}

async fn run_body<Fut>(body: Fut) -> Result<(), BodyFailure>
where
    Fut: Future<Output = ()> + Send + 'static,
{
    let mut bodies: JoinSet<()> = JoinSet::new();
    bodies.spawn(body);
    match timeout(BODY_TIMEOUT, bodies.join_next()).await {
        Ok(Some(Ok(()))) => Ok(()),
        Ok(Some(Err(error))) if error.is_panic() => {
            drop(error);
            Err(BodyFailure::Panicked)
        }
        Ok(Some(Err(_))) => panic!("body task was cancelled"),
        Ok(None) => panic!("body task ended without an outcome"),
        Err(_) => {
            bodies.abort_all();
            let drained_after_abort = timeout(SHUTDOWN_TIMEOUT, async {
                while bodies.join_next().await.is_some() {}
            })
            .await;
            if drained_after_abort.is_err() {
                bodies.abort_all();
            }
            Err(BodyFailure::TimedOut)
        }
    }
}

async fn harness<F, Fut>(body: F)
where
    F: FnOnce(Client) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let (fixture, client, tasks) = Box::pin(timeout(HARNESS_SETUP_TIMEOUT, setup()))
        .await
        .unwrap_or_else(|_| panic!("timed out setting up fixture"));
    let outcome = Box::pin(run_body(body(client))).await;
    close_fixture(tasks, fixture).await;
    if let Err(error) = outcome {
        match error {
            BodyFailure::Panicked => panic!("schema-baseline test body panicked"),
            BodyFailure::TimedOut => panic!("timed out running schema-baseline test"),
        }
    }
}

async fn harness2<F, Fut>(body: F)
where
    F: FnOnce(Client, Client) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    harness(move |client_a| nested_harness(client_a, body)).await;
}

async fn nested_harness<F, Fut>(client_a: Client, body: F)
where
    F: FnOnce(Client, Client) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    harness(move |client_b| body(client_a, client_b)).await;
}

async fn exec(client: &Client, sql: &str) {
    client.execute(sql, &[]).await.unwrap();
}

async fn exec_err(client: &Client, sql: &str) -> tokio_postgres::Error {
    client.execute(sql, &[]).await.unwrap_err()
}

async fn scalar_text(client: &Client, sql: &str) -> String {
    client.query_one(sql, &[]).await.unwrap().get(0)
}

async fn scalar_i64(client: &Client, sql: &str) -> i64 {
    client.query_one(sql, &[]).await.unwrap().get(0)
}

async fn insert_budget(client: &Client, id: &str) {
    client
        .execute(
            r#"INSERT INTO "LiteLLM_BudgetTable" (budget_id, created_by, updated_by) VALUES ($1, 't', 't')"#,
            &[&id],
        )
        .await
        .unwrap();
}

struct ColumnInfo {
    data_type: String,
    is_nullable: bool,
    default: Option<String>,
}

async fn column(client: &Client, table: &str, name: &str) -> ColumnInfo {
    let rows = columns_in(client, "public", table).await;
    let row = rows
        .iter()
        .find(|row| row.get::<_, String>(1) == name)
        .unwrap();
    ColumnInfo {
        data_type: row.get(2),
        is_nullable: row.get::<_, String>(5) == "YES",
        default: row.get(6),
    }
}

async fn query_one_row(client: &Client, sql: &str, params: &[&(dyn ToSql + Sync)]) -> Row {
    client.query_one(sql, params).await.unwrap()
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

async fn index_facts(client: &Client, index: &str) -> IndexFacts {
    let unique = query_one_row(
        client,
        "SELECT indisunique FROM pg_index idx \
         JOIN pg_class ic ON ic.oid = idx.indexrelid WHERE ic.relname = $1",
        &[&index],
    )
    .await
    .get(0);
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

fn is_zero(default: Option<String>) -> bool {
    let value: Decimal = default.unwrap().trim().parse().unwrap();
    value == Decimal::ZERO
}

type ColumnFacts = (
    i32,
    String,
    String,
    String,
    Option<i32>,
    bool,
    Option<String>,
);

async fn columns_in(client: &Client, schema: &str, table: &str) -> Vec<Row> {
    client
        .query(
            "SELECT ordinal_position, column_name, data_type, udt_name, datetime_precision, \
             is_nullable, column_default \
             FROM information_schema.columns WHERE table_schema = $1 AND table_name = $2 \
             ORDER BY ordinal_position",
            &[&schema, &table],
        )
        .await
        .unwrap()
}

async fn table_shape(
    client: &Client,
    schema: &str,
    table: &str,
) -> (Vec<ColumnFacts>, Vec<String>) {
    let columns = columns_in(client, schema, table)
        .await
        .iter()
        .map(|row| {
            (
                row.get(0),
                row.get(1),
                row.get(2),
                row.get(3),
                row.get(4),
                row.get::<_, String>(5) == "YES",
                row.get(6),
            )
        })
        .collect();
    let primary_key = string_rows(
        client,
        "SELECT kcu.column_name FROM information_schema.table_constraints tc \
         JOIN information_schema.key_column_usage kcu \
         ON tc.constraint_name = kcu.constraint_name AND tc.table_schema = kcu.table_schema \
         WHERE tc.table_schema = $1 AND tc.table_name = $2 AND tc.constraint_type = 'PRIMARY KEY' \
         ORDER BY kcu.ordinal_position",
        &[&schema, &table],
    )
    .await;
    (columns, primary_key)
}
