#![cfg(test)]

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::time::Duration;

use betterlite_config::sections::database_startup::packaged;
use betterlite_db::request_log_index_catalog::definitions::REQUEST_LOG_INDEXES;
use betterlite_db_admin::command_shell::{
    AdminEnvironment, DbAdminBackend, Diagnostics, ExitStatus, run,
};
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::task::JoinSet;
use tokio::time::timeout;
use tokio_postgres::{Client, NoTls};

const TEST_TIMEOUT: Duration = Duration::from_secs(240);

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

#[derive(Parser)]
struct TestEnvironment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

struct Postgres {
    fixture: PostgresFixture,
    client: Client,
    tasks: JoinSet<Result<(), tokio_postgres::Error>>,
    schema: String,
}

impl Postgres {
    async fn new() -> Self {
        let url = TestEnvironment::parse_from(["database-startup-admin-tests"])
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
        let schema: String = client
            .query_one("SELECT current_schema()", &[])
            .await
            .expect("current_schema reads")
            .get(0);
        Self {
            fixture,
            client,
            tasks,
            schema,
        }
    }

    async fn close(self) {
        drop(self.client);
        drop(self.tasks);
        self.fixture
            .close_async()
            .await
            .expect("the schema is dropped");
    }
}

#[tokio::test]
async fn dispatches_native_operation() {
    let database = Postgres::new().await;
    let url = format!(
        "{}&schema={}",
        database.fixture.options.url.expose_secret(),
        database.schema
    );
    let backend = crate::backend();
    let (status, lines) = timeout(TEST_TIMEOUT, run_command(&["migrate"], &url, &backend))
        .await
        .expect("the migrate command finishes");
    assert_eq!(status, ExitStatus::Success);
    assert_eq!(
        lines,
        vec![
            "err|Starting migration job (use_migrate=true, use_v2_resolver=true)".to_owned(),
            "err|Migration job completed successfully.".to_owned(),
        ]
    );
    let rows = database
        .client
        .query(
            "SELECT migration_name, finished_at IS NOT NULL FROM _prisma_migrations",
            &[],
        )
        .await
        .expect("the migration ledger reads");
    assert_eq!(rows.len(), packaged::migrations().len());
    assert!(rows.iter().all(|row| row.get::<_, bool>(1)));
    for index in REQUEST_LOG_INDEXES {
        let name = index.name.to_owned();
        let exists: bool = database
            .client
            .query_one(
                "SELECT EXISTS (SELECT 1 FROM pg_indexes \
                 WHERE schemaname = current_schema() AND indexname = $1)",
                &[&name],
            )
            .await
            .expect("the index catalog reads")
            .get(0);
        assert!(exists, "the migration job built {name}");
    }

    // a repeated command finds the schema current and leaves the ledger alone
    let (second_status, second_lines) =
        timeout(TEST_TIMEOUT, run_command(&["migrate"], &url, &backend))
            .await
            .expect("the second migrate command finishes");
    assert_eq!(second_status, ExitStatus::Success);
    assert_eq!(second_lines, lines);
    let rows = database
        .client
        .query(
            "SELECT finished_at IS NOT NULL FROM _prisma_migrations",
            &[],
        )
        .await
        .expect("the migration ledger reads");
    assert_eq!(rows.len(), packaged::migrations().len());
    assert!(rows.iter().all(|row| row.get::<_, bool>(0)));
    database.close().await;
}

#[tokio::test]
async fn rejects_bad_arguments() {
    let backend = crate::backend();
    for arguments in [
        &["migrate", "--bogus"][..],
        &["migrate", "extra"][..],
        &[
            "--config",
            "/nonexistent/betterlite.yaml",
            "migrate",
            "--bogus",
        ][..],
    ] {
        let (status, lines) =
            run_command(arguments, "postgresql://pinned.example/db", &backend).await;
        assert_eq!(status, ExitStatus::Usage, "{arguments:?}");
        assert_eq!(status.code(), 2);
        assert!(!lines.is_empty());
        assert!(lines.iter().all(|line| line.starts_with("err|")));
        assert!(
            !lines
                .iter()
                .any(|line| line.contains("Starting migration job")),
            "{arguments:?} reached the service"
        );
    }
}

#[tokio::test]
async fn migrate_reports_an_unrecoverable_failure() {
    let database = Postgres::new().await;
    database
        .client
        .batch_execute("CREATE TABLE _prisma_migrations (bogus text)")
        .await
        .expect("a broken ledger");
    let url = format!(
        "{}&schema={}",
        database.fixture.options.url.expose_secret(),
        database.schema
    );
    let (status, lines) = timeout(
        TEST_TIMEOUT,
        run_command(&["migrate"], &url, &crate::backend()),
    )
    .await
    .expect("the migrate command finishes");
    assert_eq!(status, ExitStatus::Unrecoverable);
    assert_eq!(status.code(), 2);
    assert_eq!(lines.len(), 2);
    assert!(lines[0].starts_with("err|Starting migration job"));
    assert!(
        lines[1].starts_with(
            "err|Database migration cannot proceed: Database migration failed \
             and cannot be auto-recovered."
        ),
        "{}",
        lines[1]
    );
    database.close().await;
}
