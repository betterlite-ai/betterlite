#![cfg(test)]

use std::collections::BTreeMap;
use std::ffi::OsString;

use betterlite_db_admin::command_shell::{
    AdminEnvironment, DbAdminBackend, Diagnostics, ExitStatus, run,
};
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::task::JoinSet;
use tokio_postgres::{Client, NoTls};

const SCHEMA_SQL: &str = r#"CREATE TABLE "LiteLLM_SpendLogs" (
    "request_id" TEXT NOT NULL,
    "call_type" TEXT NOT NULL,
    "api_key" TEXT NOT NULL DEFAULT '',
    "spend" DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    "total_tokens" INTEGER NOT NULL DEFAULT 0,
    "startTime" TIMESTAMP(3) NOT NULL,
    "endTime" TIMESTAMP(3) NOT NULL,
    "end_user" TEXT,
    "session_id" TEXT,
    CONSTRAINT "LiteLLM_SpendLogs_pkey" PRIMARY KEY ("request_id")
);
CREATE INDEX "LiteLLM_SpendLogs_startTime_idx" ON "LiteLLM_SpendLogs"("startTime");
CREATE INDEX "LiteLLM_SpendLogs_startTime_request_id_idx" ON "LiteLLM_SpendLogs"("startTime", "request_id");
CREATE INDEX "LiteLLM_SpendLogs_end_user_idx" ON "LiteLLM_SpendLogs"("end_user");
CREATE INDEX "LiteLLM_SpendLogs_session_id_idx" ON "LiteLLM_SpendLogs"("session_id");
CREATE INDEX "LiteLLM_SpendLogs_api_key_startTime_idx" ON "LiteLLM_SpendLogs"("api_key", "startTime");"#;

const SEED_SQL: &str = r#"INSERT INTO "LiteLLM_SpendLogs" (request_id, call_type, api_key, "startTime", "endTime") VALUES ('admin-before', 'completion', 'key-one', '2026-01-01T00:00:00', '2026-01-01T00:00:01');"#;

const LIVE_SQL: &str = r#"INSERT INTO "LiteLLM_SpendLogs" (request_id, call_type, api_key, "startTime", "endTime") VALUES ('admin-after', 'completion', 'key-two', '2026-02-01T00:00:00', '2026-02-01T00:00:01');"#;

const PARTITION_LINE: &str = "partition-spend-logs completed; the new LiteLLM_SpendLogs is empty and rows remain in LiteLLM_SpendLogs_legacy.";
const UNPARTITION_LINE: &str = "unpartition-spend-logs completed; every surviving row now lives in the plain LiteLLM_SpendLogs.";

#[derive(Parser)]
struct TestEnvironment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

struct Postgres {
    fixture: PostgresFixture,
    client: Client,
    tasks: JoinSet<Result<(), tokio_postgres::Error>>,
}

impl Postgres {
    async fn provision() -> Self {
        let url = TestEnvironment::parse_from(["spend-partitions-admin"])
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
            .expect("the pinned LiteLLM_SpendLogs schema");
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
    rejects(&["partition-spend-logs", "extra"]).await;
    rejects(&["partition-spend-logs", "--cutover", "2026-07-25T00:00:00Z"]).await;
    rejects(&["unpartition-spend-logs", "extra"]).await;
    rejects(&["unpartition-spend-logs", "--bogus"]).await;
}

#[tokio::test]
async fn preserves_operator_guidance() {
    let backend = crate::backend();
    let (status, lines) = run_command(
        &["partition-spend-logs", "--help"],
        "postgresql://pinned.example/db",
        &backend,
    )
    .await;
    assert_eq!(status, ExitStatus::Success);
    let help = lines.join("\n");
    assert!(help.contains("use_spend_logs_partitioning"));
    assert!(help.contains("maximum_spend_logs_retention_period"));
    assert!(help.contains("SPEND_LOG_PARTITION_INTERVAL"));
    assert!(help.contains("disable"));
    let (restore_status, restore_lines) = run_command(
        &["unpartition-spend-logs", "--help"],
        "postgresql://pinned.example/db",
        &backend,
    )
    .await;
    assert_eq!(restore_status, ExitStatus::Success);
    let restore_help = restore_lines.join("\n");
    assert!(restore_help.contains("use_spend_logs_partitioning"));
    assert!(restore_help.contains("duplicate"));
    assert!(restore_help.contains("disable"));
}

#[tokio::test]
async fn dispatches_native_operation() {
    let postgres = Postgres::provision().await;
    postgres
        .client
        .batch_execute(SEED_SQL)
        .await
        .expect("the spend log row");
    let url = postgres.fixture.options.url.expose_secret().to_owned();
    let backend = crate::backend();
    let (status, lines) = run_command(&["partition-spend-logs"], &url, &backend).await;
    assert_eq!(status, ExitStatus::Success);
    assert_eq!(lines, vec![format!("out|{PARTITION_LINE}")]);
    assert_converted(&postgres).await;
    let (repeat_status, repeat_lines) =
        run_command(&["partition-spend-logs"], &url, &backend).await;
    assert_eq!(repeat_status, ExitStatus::Failure);
    assert!(
        repeat_lines
            .iter()
            .any(|line| line.contains("LiteLLM_SpendLogs_legacy"))
    );
    postgres
        .client
        .batch_execute(LIVE_SQL)
        .await
        .expect("the live spend log row");
    let (restore_status, restore_lines) =
        run_command(&["unpartition-spend-logs"], &url, &backend).await;
    assert_eq!(restore_status, ExitStatus::Success);
    assert_eq!(restore_lines, vec![format!("out|{UNPARTITION_LINE}")]);
    assert_restored(&postgres).await;
    postgres.close().await;
}

async fn table_present(postgres: &Postgres, table: &str) -> bool {
    let row = postgres
        .client
        .query_one("SELECT to_regclass($1) IS NOT NULL AS present", &[&table])
        .await
        .expect("the table lookup");
    row.get("present")
}

async fn row_count(postgres: &Postgres, table: &str) -> i64 {
    let sql = format!(r#"SELECT count(*) AS rows FROM "{table}""#);
    let row = postgres
        .client
        .query_one(&sql, &[])
        .await
        .expect("a row count");
    row.get("rows")
}

async fn assert_converted(postgres: &Postgres) {
    assert!(table_present(postgres, "LiteLLM_SpendLogs_legacy").await);
    assert!(table_present(postgres, "LiteLLM_SpendLogs_pdefault").await);
    let row = postgres
        .client
        .query_one(
            "SELECT relkind::text AS relkind FROM pg_class WHERE oid = to_regclass('LiteLLM_SpendLogs')",
            &[],
        )
        .await
        .expect("the canonical table");
    assert_eq!(row.get::<_, String>("relkind"), "p");
    assert_eq!(row_count(postgres, "LiteLLM_SpendLogs").await, 0);
    assert_eq!(row_count(postgres, "LiteLLM_SpendLogs_legacy").await, 1);
}

async fn assert_restored(postgres: &Postgres) {
    assert!(!table_present(postgres, "LiteLLM_SpendLogs_partitioned").await);
    let row = postgres
        .client
        .query_one(
            "SELECT relkind::text AS relkind FROM pg_class WHERE oid = to_regclass('LiteLLM_SpendLogs')",
            &[],
        )
        .await
        .expect("the canonical table");
    assert_eq!(row.get::<_, String>("relkind"), "r");
    let rows = postgres
        .client
        .query(
            r#"SELECT request_id FROM "LiteLLM_SpendLogs" ORDER BY request_id"#,
            &[],
        )
        .await
        .expect("the restored row");
    let ids: Vec<String> = rows.iter().map(|row| row.get("request_id")).collect();
    assert_eq!(ids, vec!["admin-after".to_owned()]);
    assert_eq!(row_count(postgres, "LiteLLM_SpendLogs_legacy").await, 1);
    assert_restored_primary_key(postgres).await;
}

async fn assert_restored_primary_key(postgres: &Postgres) {
    let primary = postgres
        .client
        .query_one(
            r#"SELECT i.relname AS index_name, pg_get_indexdef(ix.indexrelid) AS definition
FROM pg_index ix
JOIN pg_class i ON i.oid = ix.indexrelid
JOIN pg_class c ON c.oid = ix.indrelid
WHERE c.relname = 'LiteLLM_SpendLogs' AND c.relnamespace = current_schema()::regnamespace AND ix.indisprimary"#,
            &[],
        )
        .await
        .expect("the primary key");
    assert_eq!(
        primary.get::<_, String>("index_name"),
        "LiteLLM_SpendLogs_pkey"
    );
    assert!(
        primary
            .get::<_, String>("definition")
            .contains("(request_id)")
    );
}
