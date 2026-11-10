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

use super::{OFFLINE_TIMEOUT, offline_views};

const PERMISSION_DENIED: &str = "permission denied for table LiteLLM_VerificationToken";
const FAST: Duration = Duration::from_secs(5);

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
    ) -> Pin<Box<dyn Future<Output = Result<Value, String>> + Send + 'database>>
    where
        Value: Default + Send + 'database,
    {
        let (label, hang, failure) = match call {
            Call::Probe => {
                let probe = |name: &&str| query == format!("SELECT 1 FROM \"{name}\" LIMIT 1");
                let exists = self.existing.iter().any(probe);
                let failure = (!exists).then(|| self.probe_error.to_owned());
                ("probe", self.hang_probe, failure)
            }
            Call::Create => {
                let view = OFFLINE_VIEWS.iter().find(|view| view.ddl == query);
                let message = view.and_then(|found| self.create_errors.get(found.name));
                let failure = message.map(|text| (*text).to_owned());
                ("create", self.hang_create, failure)
            }
        };
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
    type Error = String;

    fn execute_raw<'database>(
        &'database self,
        query: &'database str,
    ) -> ExecuteFuture<'database, String> {
        self.answer(Call::Create, query)
    }
}

impl SupportsRawQueries for Scripted {
    fn query_raw<'database>(
        &'database self,
        query: &'database str,
    ) -> QueryFuture<'database, String> {
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

#[test]
fn offline_timeout_sixty_thousand_seconds() {
    assert_eq!(OFFLINE_TIMEOUT.as_secs(), 60_000);
}

#[tokio::test]
async fn offline_deadline_bounds_each_round_trip() {
    let mut hanging_create = Scripted::new("does not exist", &[]);
    hanging_create.hang_create = true;
    let (lines, source) =
        views_failure(offline_views(&hanging_create, Duration::from_secs(1)).await);
    assert!(lines.is_empty());
    assert!(matches!(source, ViewsError::Timeout { seconds: 1 }));
    let mut hanging_probe = Scripted::new("", &[]);
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
    let mut database = Scripted::new("relation does not exist", &[]);
    database.create_errors.insert(
        "Last30dKeysBySpend",
        "relation \"Last30dKeysBySpend\" already exists",
    );
    let (lines, source) = views_failure(offline_views(&database, FAST).await);
    assert_eq!(lines, lines_of(&names()[..2], "Created!"));
    assert!(
        matches!(source, ViewsError::Database { message } if message.contains("already exists"))
    );
    let creates = database
        .calls()
        .iter()
        .filter(|call| call.starts_with("create|"))
        .count();
    assert_eq!(creates, 3);
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

struct Recorder(Vec<String>);

impl Diagnostics for Recorder {
    fn output(&mut self, line: &str) {
        self.0.push(format!("out|{line}"));
    }

    fn error(&mut self, line: &str) {
        self.0.push(format!("err|{line}"));
    }
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
    let environment = AdminEnvironment {
        variables: BTreeMap::from([(
            "DATABASE_URL".to_owned(),
            "postgresql://pinned.example/db".to_owned(),
        )]),
        home: None,
    };
    let mut argv = vec![OsString::from("betterlite-db-admin")];
    argv.extend(arguments.iter().map(OsString::from));
    let mut recorder = Recorder(Vec::new());
    let status = run(argv, environment, &backend, &mut recorder).await;
    (status, recorder.0, harness)
}

#[tokio::test]
async fn dispatches_native_operation() {
    let (status, lines, harness) = invoke(&["database-views"], Scripted::default()).await;
    assert_eq!(status, ExitStatus::Success);
    assert_eq!(
        *harness.commands.lock().expect("commands"),
        vec![DbAdminCommand::DatabaseViews]
    );
    assert_eq!(lines.len(), 8);
    assert!(lines.iter().all(|line| line.starts_with("out|")));
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
        "does not exist",
        &["MonthlyGlobalSpend", "Last30dTopEndUsersSpend"],
    );
    let (status, lines, _) = invoke(&["database-views"], existing).await;
    assert_eq!(status.code(), 0);
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
    let mut failing = Scripted::new("does not exist", &[]);
    failing
        .create_errors
        .insert("MonthlyGlobalSpend", "permission denied for schema public");
    let (failed_status, failed_lines, harness) = invoke(&["database-views"], failing).await;
    assert_eq!(failed_status, ExitStatus::Failure);
    assert_eq!(failed_status.code(), 1);
    assert_eq!(
        failed_lines,
        vec![
            "out|LiteLLM_VerificationTokenView Created!".to_owned(),
            "err|permission denied for schema public".to_owned(),
        ]
    );
    assert_eq!(harness.database.calls().len(), 4);
}
