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
}

impl Postgres {
    async fn new() -> Self {
        let url = TestEnvironment::parse_from(["database-startup-admin-tests"])
            .url
            .map(SecretString::from);
        let fixture = PostgresFixture::provision_with_statements(url.as_ref(), Vec::new())
            .resolve()
            .await
            .expect("a test database");
        let (client, connection) =
            tokio_postgres::connect(fixture.options.url.expose_secret(), NoTls)
                .await
                .expect("a test connection");
        let mut tasks = JoinSet::new();
        tasks.spawn(connection);
        Self {
            fixture,
            client,
            tasks,
        }
    }

    async fn close(self) {
        self.fixture
            .close_async()
            .await
            .expect("the database is dropped");
        drop(self.client);
        drop(self.tasks);
    }
}

#[tokio::test]
async fn dispatches_native_operation() {
    let database = Postgres::new().await;
    let url = database.fixture.options.url.expose_secret();
    let (status, lines) = run_migrate(url).await;
    assert_eq!(status, ExitStatus::Success);
    assert_eq!(
        lines,
        vec![
            "err|Starting migration job (use_migrate=true, use_v2_resolver=true)".to_owned(),
            "err|Migration job completed successfully.".to_owned(),
        ]
    );
    assert_eq!(
        ledger_rows(&database).await.len(),
        packaged::migrations().len()
    );
    assert!(ledger_rows_are_finished(&database).await);
    assert_eq!(
        present_indexes(&database).await,
        expected_indexes(),
        "the migration job built every request-log index"
    );

    // a repeated command finds the schema current and leaves the ledger alone
    let (second_status, second_lines) = run_migrate(url).await;
    assert_eq!(second_status, ExitStatus::Success);
    assert_eq!(second_lines, lines);
    assert_eq!(
        ledger_rows(&database).await.len(),
        packaged::migrations().len()
    );
    assert!(ledger_rows_are_finished(&database).await);
    database.close().await;
}

async fn run_migrate(url: &str) -> (ExitStatus, Vec<String>) {
    let backend = crate::backend();
    timeout(TEST_TIMEOUT, run_command(&["migrate"], url, &backend))
        .await
        .expect("the migrate command finishes")
}

async fn ledger_rows(database: &Postgres) -> Vec<tokio_postgres::Row> {
    database
        .client
        .query(
            "SELECT migration_name, finished_at IS NOT NULL FROM _prisma_migrations",
            &[],
        )
        .await
        .expect("the migration ledger reads")
}

async fn ledger_rows_are_finished(database: &Postgres) -> bool {
    ledger_rows(database)
        .await
        .iter()
        .all(|row| row.get::<_, bool>(1))
}

async fn present_indexes(database: &Postgres) -> Vec<String> {
    let names: Vec<&str> = REQUEST_LOG_INDEXES.iter().map(|index| index.name).collect();
    let mut present: Vec<String> = database
        .client
        .query(
            "SELECT indexname::text FROM pg_indexes \
             WHERE schemaname = current_schema() AND indexname::text = ANY($1)",
            &[&names],
        )
        .await
        .expect("the index catalog reads")
        .iter()
        .map(|row| row.get(0))
        .collect();
    present.sort();
    present
}

fn expected_indexes() -> Vec<String> {
    let mut expected: Vec<String> = REQUEST_LOG_INDEXES
        .iter()
        .map(|index| index.name.to_owned())
        .collect();
    expected.sort();
    expected
}

#[rstest::rstest]
#[case::unknown_flag(&["migrate", "--bogus"])]
#[case::unexpected_positional(&["migrate", "extra"])]
#[case::unknown_flag_after_config(&[
    "--config",
    "/nonexistent/betterlite.yaml",
    "migrate",
    "--bogus"
])]
#[tokio::test]
async fn rejects_bad_arguments(#[case] arguments: &[&str]) {
    let backend = crate::backend();
    let (status, lines) = run_command(arguments, "postgresql://pinned.example/db", &backend).await;
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

#[tokio::test]
async fn migrate_reports_an_unrecoverable_failure() {
    let database = Postgres::new().await;
    database
        .client
        .batch_execute("CREATE TABLE _prisma_migrations (bogus text)")
        .await
        .expect("a broken ledger");
    let url = database.fixture.options.url.expose_secret();
    let (status, lines) = timeout(
        TEST_TIMEOUT,
        run_command(&["migrate"], url, &crate::backend()),
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
