#![cfg(test)]

use std::collections::BTreeMap;
use std::ffi::OsString;

use betterlite_db::spend_backfills::BackfillError;
use betterlite_db_admin::command_shell::{
    AdminEnvironment, DbAdminBackend, Diagnostics, ExitStatus, run,
};
use betterlite_db_admin::commands::DbAdminError;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::task::JoinSet;
use tokio_postgres::{Client, Config, NoTls};

const SCHEMA_SQL: &str = r#"CREATE TABLE "LiteLLM_BudgetTable" (
    "budget_id" TEXT NOT NULL,
    "budget_duration" TEXT,
    CONSTRAINT "LiteLLM_BudgetTable_pkey" PRIMARY KEY ("budget_id")
);
CREATE TABLE "LiteLLM_VerificationToken" (
    "token" TEXT NOT NULL,
    "spend" DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    "budget_duration" TEXT,
    "budget_id" TEXT,
    "total_spend" DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    CONSTRAINT "LiteLLM_VerificationToken_pkey" PRIMARY KEY ("token")
);
CREATE TABLE "LiteLLM_DeletedVerificationToken" (
    "id" TEXT NOT NULL,
    "token" TEXT NOT NULL,
    "spend" DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    "budget_duration" TEXT,
    "budget_id" TEXT,
    "total_spend" DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    CONSTRAINT "LiteLLM_DeletedVerificationToken_pkey" PRIMARY KEY ("id")
);
CREATE TABLE "LiteLLM_SpendLogs" (
    "request_id" TEXT NOT NULL,
    "api_key" TEXT NOT NULL DEFAULT '',
    "spend" DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    "total_tokens" INTEGER NOT NULL DEFAULT 0,
    "startTime" TIMESTAMP(3) NOT NULL,
    CONSTRAINT "LiteLLM_SpendLogs_pkey" PRIMARY KEY ("request_id")
);
CREATE TABLE "LiteLLM_SpendLogToolIndex" (
    "request_id" TEXT NOT NULL,
    "tool_name" TEXT NOT NULL,
    "start_time" TIMESTAMP(3) NOT NULL,
    CONSTRAINT "LiteLLM_SpendLogToolIndex_pkey" PRIMARY KEY ("request_id","tool_name")
);
CREATE TABLE "LiteLLM_DailyToolSpend" (
    "date" TEXT NOT NULL,
    "tool_name" TEXT NOT NULL,
    "spend" DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    "total_tokens" BIGINT NOT NULL DEFAULT 0,
    "request_count" BIGINT NOT NULL DEFAULT 0,
    "created_at" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "updated_at" TIMESTAMP(3) NOT NULL,
    CONSTRAINT "LiteLLM_DailyToolSpend_pkey" PRIMARY KEY ("date","tool_name")
);"#;

const SEED_SQL: &str = r#"INSERT INTO "LiteLLM_SpendLogs" (request_id, api_key, spend, total_tokens, "startTime") VALUES ('request-one', 'key-one', 9.0, 10, '2026-07-01T00:00:00');
INSERT INTO "LiteLLM_SpendLogToolIndex" (request_id, tool_name, start_time) VALUES ('request-one', 'tool-a', '2026-06-01T00:00:00');
INSERT INTO "LiteLLM_VerificationToken" (token, spend, total_spend, budget_duration) VALUES ('key-one', 5.0, 1.0, '1d');
INSERT INTO "LiteLLM_DeletedVerificationToken" (id, token, spend, total_spend) VALUES ('life-one', 'key-one', 7.0, 2.0);"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

struct Postgres {
    fixture: PostgresFixture,
    client: Client,
    tasks: JoinSet<Result<(), tokio_postgres::Error>>,
}

impl Postgres {
    async fn new() -> Self {
        let url = Environment::parse_from(["spend-backfills-admin"])
            .url
            .map(SecretString::from);
        let fixture = PostgresFixture::provision_schema(url.as_ref())
            .resolve()
            .await
            .expect("a test schema");
        let (client, connection) =
            tokio_postgres::connect(fixture.options.url.expose_secret(), NoTls)
                .await
                .expect("a test connection");
        let mut tasks = JoinSet::new();
        tasks.spawn(connection);
        client
            .batch_execute(SCHEMA_SQL)
            .await
            .expect("the pinned tables");
        Self {
            fixture,
            client,
            tasks,
        }
    }

    async fn close(self) {
        let Self {
            fixture,
            client,
            tasks,
        } = self;
        drop(client);
        drop(tasks);
        fixture.close_async().await.expect("the schema is dropped");
    }
}

struct Recorder(Vec<String>);

impl Diagnostics for Recorder {
    fn output(&mut self, line: &str) {
        self.0.push(format!("out|{line}"));
    }

    fn error(&mut self, line: &str) {
        self.0.push(format!("err|{line}"));
    }
}

async fn run_command(
    arguments: &[&str],
    database_url: &str,
    backend: &DbAdminBackend,
) -> (ExitStatus, Vec<String>) {
    let environment = AdminEnvironment {
        variables: BTreeMap::from([("DATABASE_URL".to_owned(), database_url.to_owned())]),
        home: None,
    };
    let mut argv = vec![OsString::from("betterlite-db-admin")];
    argv.extend(arguments.iter().map(OsString::from));
    let mut recorder = Recorder(Vec::new());
    let status = Box::pin(run(argv, environment, backend, &mut recorder)).await;
    (status, recorder.0)
}

async fn rejects(arguments: &[&str]) {
    let backend = crate::backend();
    let (status, lines) = run_command(arguments, "postgresql://pinned.example/db", &backend).await;
    assert_eq!(status, ExitStatus::Usage);
    assert_eq!(status.code(), 2);
    assert!(lines.iter().all(|line| line.starts_with("err|")));
    assert!(!lines.iter().any(|line| line.contains("configuration")));
}

#[tokio::test]
async fn rejects_bad_arguments() {
    rejects(&["daily-tool-spend"]).await;
    rejects(&["daily-tool-spend", "--cutover", "2026-07-25T00:00:00"]).await;
    rejects(&["daily-tool-spend", "--cutover", "not-a-timestamp"]).await;
    rejects(&[
        "daily-tool-spend",
        "--cutover",
        "2026-07-25T00:00:00Z",
        "extra",
    ])
    .await;
    rejects(&["key-total-spend", "--bogus"]).await;
    rejects(&["key-total-spend-from-logs", "extra"]).await;
}

#[tokio::test]
async fn names_the_unregistered_backfill_operation() {
    let backend = DbAdminBackend::default();
    let (status, lines) = run_command(
        &["key-total-spend"],
        "postgresql://pinned.example/db",
        &backend,
    )
    .await;
    assert_eq!(status, ExitStatus::Failure);
    assert!(lines.iter().any(|line| {
        line.contains("no native service is registered for the key-total-spend operation")
    }));
}

#[test]
fn maps_statement_failures_to_the_admin_error() {
    let error = BackfillError::Statement {
        statement: "the daily insert",
        source: driver_error(),
    };
    let mapped = DbAdminError::from(error);
    assert!(mapped.to_string().contains("the daily insert failed:"));
}

fn driver_error() -> tokio_postgres::Error {
    "unknown_option=on"
        .parse::<Config>()
        .expect_err("an unknown connection option")
}

#[tokio::test]
async fn exposes_operator_caveats() {
    let (daily_status, daily_lines) = run_command(
        &["daily-tool-spend", "--help"],
        "postgresql://pinned.example/db",
        &crate::backend(),
    )
    .await;
    assert_eq!(daily_status, ExitStatus::Success);
    let daily_help = daily_lines.join("\n");
    assert!(daily_help.contains("declared but never invoked"));
    assert!(daily_help.contains("retention"));
    let (history_status, history_lines) = run_command(
        &["key-total-spend-from-logs", "--help"],
        "postgresql://pinned.example/db",
        &crate::backend(),
    )
    .await;
    assert_eq!(history_status, ExitStatus::Success);
    let history_help = history_lines.join("\n");
    assert!(history_help.contains("Run key-total-spend first"));
    assert!(history_help.contains("retention"));
    assert!(history_help.contains("traffic paused"));
}

#[tokio::test]
async fn dispatches_native_operation() {
    let postgres = Postgres::new().await;
    postgres
        .client
        .batch_execute(SEED_SQL)
        .await
        .expect("the backfill rows");
    let url = postgres.fixture.options.url.expose_secret().to_owned();
    let backend = crate::backend();
    dispatch_each(&url, &backend).await;
    assert_rows(&postgres).await;
    postgres.close().await;
}

async fn dispatch_each(url: &str, backend: &DbAdminBackend) {
    let (daily_status, daily_lines) = run_command(
        &["daily-tool-spend", "--cutover", "2026-07-25T00:00:00Z"],
        url,
        backend,
    )
    .await;
    assert_eq!(daily_status, ExitStatus::Success);
    assert_eq!(
        daily_lines,
        vec!["out|daily-tool-spend inserted 1 buckets.".to_owned()]
    );
    let (floor_status, floor_lines) = run_command(&["key-total-spend"], url, backend).await;
    assert_eq!(floor_status, ExitStatus::Success);
    assert_eq!(
        floor_lines,
        vec!["out|key-total-spend updated 1 active and 1 archived keys.".to_owned()]
    );
    let (history_status, history_lines) =
        run_command(&["key-total-spend-from-logs"], url, backend).await;
    assert_eq!(history_status, ExitStatus::Success);
    assert_eq!(
        history_lines,
        vec!["out|key-total-spend-from-logs updated 1 active and 0 archived keys.".to_owned()]
    );
}

async fn assert_rows(postgres: &Postgres) {
    let bucket = postgres
        .client
        .query_one(
            r#"SELECT spend, total_tokens, request_count FROM "LiteLLM_DailyToolSpend" WHERE date = '2026-06-01' AND tool_name = 'tool-a'"#,
            &[],
        )
        .await
        .expect("the backfilled bucket");
    assert_eq!(decimal(bucket.get("spend")), Decimal::new(9, 0));
    assert_eq!(bucket.get::<_, i64>("total_tokens"), 10);
    assert_eq!(bucket.get::<_, i64>("request_count"), 1);
    let active = postgres
        .client
        .query_one(
            r#"SELECT total_spend FROM "LiteLLM_VerificationToken" WHERE token = 'key-one'"#,
            &[],
        )
        .await
        .expect("the active key");
    assert_eq!(decimal(active.get("total_spend")), Decimal::new(9, 0));
    let archived = postgres
        .client
        .query_one(
            r#"SELECT total_spend FROM "LiteLLM_DeletedVerificationToken" WHERE id = 'life-one'"#,
            &[],
        )
        .await
        .expect("the archived key");
    assert_eq!(decimal(archived.get("total_spend")), Decimal::new(7, 0));
}

fn decimal(amount: f64) -> Decimal {
    Decimal::try_from(amount).expect("a finite amount")
}
