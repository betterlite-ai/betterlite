#![cfg(test)]

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::future::{Future, pending};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_db::database_views::contracts::{
    ExecuteFuture, QueryFuture, SupportsExecuteRaw, SupportsRawQueries,
};
use betterlite_db::database_views::definitions::{
    OFFLINE_VERIFICATION_VIEW_SQL, OFFLINE_VIEWS, VERIFICATION_TOKEN_VIEW,
};
use betterlite_db_admin::command_shell::{
    AdminEnvironment, DbAdminBackend, DbAdminFuture, DbAdminService, Diagnostics, ExitStatus,
    ResolvedConfig, run,
};
use betterlite_db_admin::commands::{
    DbAdminCommand, DbAdminError, DbAdminOutcome, Operation, ViewsError,
};
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio::time::{Instant, timeout};
use tokio_postgres::{Client, Config, NoTls};

use super::{DatabaseViewsService, OFFLINE_TIMEOUT, offline_views};

const PERMISSION_DENIED: &str = "permission_denied";
const FAST: Duration = Duration::from_secs(5);
const BASE_TABLES: &str = r#"CREATE TABLE "LiteLLM_VerificationToken" (token TEXT PRIMARY KEY, key_alias TEXT, key_name TEXT, team_id TEXT);
CREATE TABLE "LiteLLM_TeamTable" (team_id TEXT PRIMARY KEY, spend DOUBLE PRECISION, max_budget DOUBLE PRECISION, tpm_limit BIGINT, rpm_limit BIGINT);
CREATE TABLE "LiteLLM_SpendLogs" (request_id TEXT PRIMARY KEY, api_key TEXT, spend DOUBLE PRECISION, "startTime" TIMESTAMP(3), model TEXT, "user" TEXT, request_tags JSONB, end_user TEXT);"#;

#[derive(Clone, Copy)]
enum Call {
    Probe,
    Create,
}

#[derive(Default)]
struct Scripted {
    existing: Vec<&'static str>,
    probe_error: &'static str,
    create_errors: BTreeMap<&'static str, &'static str>,
    hang_probe: bool,
    hang_create: bool,
    calls: Mutex<Vec<String>>,
}

impl Scripted {
    fn new(probe_error: &'static str, existing: &[&'static str]) -> Self {
        Self {
            existing: existing.to_vec(),
            probe_error,
            create_errors: BTreeMap::new(),
            hang_probe: false,
            hang_create: false,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn answer<'database, Value>(
        &'database self,
        call: Call,
        query: &str,
    ) -> Pin<Box<dyn Future<Output = Result<Value, tokio_postgres::Error>> + Send + 'database>>
    where
        Value: Default + Send + 'database,
    {
        let (label, hang, tag) = match call {
            Call::Probe => {
                let probe = |name: &&str| query == format!("SELECT 1 FROM \"{name}\" LIMIT 1");
                let exists = self.existing.iter().any(probe);
                (
                    "probe",
                    self.hang_probe,
                    (!exists).then_some(self.probe_error),
                )
            }
            Call::Create => {
                let view = OFFLINE_VIEWS.iter().find(|view| view.ddl == query);
                let tag = view.and_then(|found| self.create_errors.get(found.name).copied());
                ("create", self.hang_create, tag)
            }
        };
        let failure = tag.map(|option| {
            format!("{option}=on")
                .parse::<Config>()
                .expect_err("an unknown connection option")
        });
        self.calls
            .lock()
            .expect("calls")
            .push(format!("{label}|{query}"));
        Box::pin(async move {
            if hang {
                pending::<()>().await;
            }
            failure.map_or_else(|| Ok(Value::default()), Err)
        })
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().expect("calls").clone()
    }
}

impl SupportsExecuteRaw for Scripted {
    type Error = tokio_postgres::Error;

    fn execute_raw<'database>(
        &'database self,
        query: &'database str,
    ) -> ExecuteFuture<'database, Self::Error> {
        self.answer(Call::Create, query)
    }
}

impl SupportsRawQueries for Scripted {
    fn query_raw<'database>(
        &'database self,
        query: &'database str,
    ) -> QueryFuture<'database, Self::Error> {
        self.answer(Call::Probe, query)
    }
}

fn names() -> Vec<&'static str> {
    OFFLINE_VIEWS.iter().map(|view| view.name).collect()
}

fn lines_of(prefix_names: &[&str], suffix: &str) -> Vec<String> {
    prefix_names
        .iter()
        .map(|name| format!("{name} {suffix}"))
        .collect()
}

fn views_failure(outcome: Result<DbAdminOutcome, DbAdminError>) -> (Vec<String>, ViewsError) {
    match outcome {
        Err(DbAdminError::Views { lines, source }) => (lines, source),
        other => panic!("expected a views failure, got {other:?}"),
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
    // run's future sits near clippy's large_futures limit, and a bigger config pushes callers past it
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
        let url = TestEnvironment::parse_from(["database-views-tests"])
            .url
            .map(SecretString::from);
        let fixture = PostgresFixture::provision_schema(url.as_ref())
            .resolve()
            .await
            .expect("a test schema");
        let (client, tasks) = Self::connect(&fixture).await;
        client
            .batch_execute(BASE_TABLES)
            .await
            .expect("the base tables");
        Self {
            fixture,
            client,
            tasks,
        }
    }

    async fn connect(
        fixture: &PostgresFixture,
    ) -> (Client, JoinSet<Result<(), tokio_postgres::Error>>) {
        let (client, connection) =
            tokio_postgres::connect(fixture.options.url.expose_secret(), NoTls)
                .await
                .expect("a test connection");
        let mut tasks = JoinSet::new();
        tasks.spawn(connection);
        (client, tasks)
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
    let url = database.fixture.options.url.expose_secret().to_owned();
    let backend = crate::backend();
    let (first_status, first_lines) = run_command(&["database-views"], &url, &backend).await;
    assert_eq!(first_status, ExitStatus::Success);
    let created: Vec<String> = lines_of(&names(), "Created!")
        .into_iter()
        .map(|line| format!("out|{line}"))
        .collect();
    assert_eq!(first_lines, created);
    let (second_status, second_lines) = run_command(&["database-views"], &url, &backend).await;
    assert_eq!(second_status, ExitStatus::Success);
    let existing: Vec<String> = lines_of(&names(), "Exists!")
        .into_iter()
        .map(|line| format!("out|{line}"))
        .collect();
    assert_eq!(second_lines, existing);
    let views = database
        .client
        .query(
            "SELECT relname::text FROM pg_class \
             WHERE relnamespace = current_schema()::regnamespace AND relkind = 'v'",
            &[],
        )
        .await
        .expect("the catalog");
    assert_eq!(views.len(), 8);
    database.close().await;
}

#[tokio::test(start_paused = true)]
async fn offline_timeout_sixty_thousand_seconds() {
    let silent = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a local port");
    let port = silent.local_addr().expect("the bound address").port();
    let url = format!("postgresql://views@127.0.0.1:{port}/views");
    let started = Instant::now();
    let (status, lines) = run_command(&["database-views"], &url, &crate::backend()).await;
    let waited = started.elapsed();
    assert!(waited >= OFFLINE_TIMEOUT, "waited {waited:?}");
    assert!(
        waited < OFFLINE_TIMEOUT + Duration::from_secs(1),
        "waited {waited:?}"
    );
    assert_eq!(status, ExitStatus::Failure);
    assert_eq!(
        lines,
        vec!["err|the database did not answer within 60000 seconds".to_owned()]
    );
    drop(silent);
}

#[tokio::test]
async fn service_deadline_bounds_each_statement() {
    let database = Postgres::new().await;
    let (locker, locker_tasks) = Postgres::connect(&database.fixture).await;
    locker
        .batch_execute(r#"BEGIN; LOCK TABLE "LiteLLM_VerificationToken" IN ACCESS EXCLUSIVE MODE;"#)
        .await
        .expect("the lock");
    let mut backend = DbAdminBackend::default();
    let service = DatabaseViewsService {
        deadline: Duration::from_secs(1),
    };
    backend
        .services
        .insert(Operation::DatabaseViews, Arc::new(service));
    let url = database.fixture.options.url.expose_secret().to_owned();
    let (status, lines) = timeout(
        Duration::from_secs(30),
        run_command(&["database-views"], &url, &backend),
    )
    .await
    .expect("the service deadline ends the blocked CREATE");
    assert_eq!(status, ExitStatus::Failure);
    assert_eq!(
        lines,
        vec!["err|the database did not answer within 1 seconds".to_owned()]
    );
    locker.batch_execute("ROLLBACK").await.expect("the unlock");
    drop(locker);
    drop(locker_tasks);
    database.close().await;
}

#[tokio::test]
async fn offline_deadline_bounds_each_round_trip() {
    let mut hanging_create = Scripted::new("relation_missing", &[]);
    hanging_create.hang_create = true;
    let (lines, source) =
        views_failure(offline_views(&hanging_create, Duration::from_secs(1)).await);
    assert!(lines.is_empty());
    assert!(matches!(source, ViewsError::Timeout { seconds: 1 }));
    let mut hanging_probe = Scripted::new("relation_missing", &[]);
    hanging_probe.hang_probe = true;
    let outcome = offline_views(&hanging_probe, Duration::from_millis(20)).await;
    let expected = DbAdminOutcome::Views {
        lines: lines_of(&names(), "Created!"),
    };
    assert_eq!(
        outcome.expect("every probe times out and creates"),
        expected
    );
}

#[tokio::test]
async fn offline_probe_error_still_attempts_create() {
    let database = Scripted::new(PERMISSION_DENIED, &[]);
    let outcome = offline_views(&database, FAST).await.expect("all created");
    assert_eq!(
        outcome,
        DbAdminOutcome::Views {
            lines: lines_of(&names(), "Created!")
        }
    );
    let expected: Vec<String> = OFFLINE_VIEWS
        .iter()
        .flat_map(|view| {
            [
                format!("probe|SELECT 1 FROM \"{}\" LIMIT 1", view.name),
                format!("create|{}", view.ddl),
            ]
        })
        .collect();
    assert_eq!(database.calls(), expected);
}

#[tokio::test]
async fn offline_duplicate_create_is_fatal() {
    let database = Postgres::new().await;
    database
        .client
        .batch_execute(
            r#"CREATE FUNCTION views_probe_refused() RETURNS integer LANGUAGE plpgsql VOLATILE AS $$ BEGIN RAISE EXCEPTION 'probe refused'; END $$;
CREATE VIEW "Last30dTopEndUsersSpend" AS SELECT 1 AS refused WHERE views_probe_refused() = 1;"#,
        )
        .await
        .expect("a view whose probe fails");
    let url = database.fixture.options.url.expose_secret().to_owned();
    let (status, lines) = run_command(&["database-views"], &url, &crate::backend()).await;
    assert_eq!(status, ExitStatus::Failure);
    let mut expected: Vec<String> = lines_of(&names()[..7], "Created!")
        .into_iter()
        .map(|line| format!("out|{line}"))
        .collect();
    expected.push(
        r#"err|a view statement failed: db error: ERROR: relation "Last30dTopEndUsersSpend" already exists"#
            .to_owned(),
    );
    assert_eq!(lines, expected);
    let kept: String = database
        .client
        .query_one(
            r#"SELECT pg_get_viewdef('"Last30dTopEndUsersSpend"'::regclass) AS definition"#,
            &[],
        )
        .await
        .expect("the existing view")
        .get("definition");
    assert!(kept.contains("views_probe_refused"));
    database.close().await;
}

#[tokio::test]
async fn offline_views_are_not_runtime_upgraded() {
    let database = Scripted::new(PERMISSION_DENIED, &[]);
    offline_views(&database, FAST).await.expect("all created");
    let verification = format!("create|{OFFLINE_VERIFICATION_VIEW_SQL}");
    assert_eq!(database.calls()[1], verification);
    assert!(!OFFLINE_VERIFICATION_VIEW_SQL.contains("team_model_max_budget"));
    assert!(!OFFLINE_VERIFICATION_VIEW_SQL.contains("project_alias"));
    assert_eq!(names()[0], VERIFICATION_TOKEN_VIEW);
}

struct ViewsHarness {
    database: Scripted,
    commands: Mutex<Vec<DbAdminCommand>>,
}

impl DbAdminService for ViewsHarness {
    fn run(&self, command: DbAdminCommand, _config: ResolvedConfig) -> DbAdminFuture<'_> {
        self.commands.lock().expect("commands").push(command);
        Box::pin(offline_views(&self.database, FAST))
    }
}

async fn invoke(
    arguments: &[&str],
    database: Scripted,
) -> (ExitStatus, Vec<String>, Arc<ViewsHarness>) {
    let harness = Arc::new(ViewsHarness {
        database,
        commands: Mutex::new(Vec::new()),
    });
    let service: Arc<dyn DbAdminService> = Arc::<ViewsHarness>::clone(&harness);
    let mut backend = DbAdminBackend::default();
    backend.services.insert(Operation::DatabaseViews, service);
    let (status, lines) = run_command(arguments, "postgresql://pinned.example/db", &backend).await;
    (status, lines, harness)
}

async fn assert_rejected(arguments: &[&str]) {
    let (status, lines, harness) = invoke(arguments, Scripted::default()).await;
    assert_eq!(status, ExitStatus::Usage);
    assert_eq!(status.code(), 2);
    assert!(harness.commands.lock().expect("commands").is_empty());
    assert!(harness.database.calls().is_empty());
    assert!(lines.iter().all(|line| line.starts_with("err|")));
    assert!(!lines.iter().any(|line| line.contains("configuration")));
}

#[tokio::test]
async fn rejects_bad_arguments() {
    assert_rejected(&["database-views", "--bogus"]).await;
    assert_rejected(&["database-views", "extra"]).await;
    let missing_config = "/nonexistent/betterlite.yaml";
    assert_rejected(&["--config", missing_config, "database-views", "--bogus"]).await;
}

#[tokio::test]
async fn offline_stdout_order_and_exit_status() {
    let existing = Scripted::new(
        "relation_missing",
        &["MonthlyGlobalSpend", "Last30dTopEndUsersSpend"],
    );
    let (status, lines, harness) = invoke(&["database-views"], existing).await;
    assert_eq!(status.code(), 0);
    assert_eq!(
        *harness.commands.lock().expect("commands"),
        vec![DbAdminCommand::DatabaseViews]
    );
    let expected: Vec<String> = names()
        .iter()
        .map(|name| {
            let state = if matches!(*name, "MonthlyGlobalSpend" | "Last30dTopEndUsersSpend") {
                "Exists!"
            } else {
                "Created!"
            };
            format!("out|{name} {state}")
        })
        .collect();
    assert_eq!(lines, expected);
    let mut failing = Scripted::new("relation_missing", &[]);
    failing
        .create_errors
        .insert("MonthlyGlobalSpend", "permission_denied");
    let (failed_status, failed_lines, failed) = invoke(&["database-views"], failing).await;
    assert_eq!(failed_status, ExitStatus::Failure);
    assert_eq!(failed_status.code(), 1);
    assert_eq!(
        failed_lines,
        vec![
            "out|LiteLLM_VerificationTokenView Created!".to_owned(),
            "err|a view statement failed: invalid connection string".to_owned(),
        ]
    );
    assert_eq!(failed.database.calls().len(), 4);
}
