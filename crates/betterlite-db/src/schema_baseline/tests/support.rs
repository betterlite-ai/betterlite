use betterlite_postgres_fixture::connection::{PostgresFixture, RandomDatabaseName};
use betterlite_postgres_fixture::seed::FixtureSeed;
use clap::Parser;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use std::future::Future;
use std::panic;
use std::str;
use tokio::task::{JoinError, JoinSet};
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

async fn setup() -> (PostgresFixture, Client, ConnectionTasks) {
    let url = postgres_test_url();
    let mut generator = RandomDatabaseName;
    let fixture = PostgresFixture::provision_with_name_generator(
        url.as_ref(),
        &FixtureSeed::default(),
        &mut generator,
    )
    .resolve()
    .await
    .unwrap();
    let (client, connection) =
        tokio_postgres::connect(fixture.options.url.expose_secret(), tokio_postgres::NoTls)
            .await
            .unwrap();
    let mut tasks = JoinSet::new();
    tasks.spawn(connection);
    client
        .batch_execute("DROP SCHEMA public CASCADE; CREATE SCHEMA public;")
        .await
        .unwrap();
    (fixture, client, tasks)
}

async fn close_fixture(mut tasks: ConnectionTasks, fixture: PostgresFixture) {
    drop(tasks.join_next().await);
    fixture.close_async().await.unwrap();
}

async fn run_body<Fut>(body: Fut) -> Result<(), JoinError>
where
    Fut: Future<Output = ()> + Send + 'static,
{
    let mut bodies: JoinSet<()> = JoinSet::new();
    bodies.spawn(body);
    bodies.join_next().await.unwrap()
}

async fn harness<F, Fut>(body: F)
where
    F: FnOnce(Client) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let (fixture, client, tasks) = setup().await;
    let outcome = run_body(body(client)).await;
    close_fixture(tasks, fixture).await;
    if let Err(err) = outcome {
        panic::resume_unwind(err.into_panic());
    }
}

async fn harness2<F, Fut>(body: F)
where
    F: FnOnce(Client, Client) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    harness(move |client_a| async move {
        harness(move |client_b| body(client_a, client_b)).await;
    })
    .await;
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
