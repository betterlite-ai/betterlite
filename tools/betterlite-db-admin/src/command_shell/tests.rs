#![cfg(test)]

use std::ffi::OsString;
use std::fs;
use std::os::unix::ffi::OsStringExt as _;
use std::sync::{Arc, Mutex};

use betterlite_db::migration_deploy::DeployError;
use rstest::rstest;
use secrecy::ExposeSecret as _;
use tempfile::TempDir;

use super::{
    AdminEnvironment, DbAdminBackend, DbAdminFuture, DbAdminService, Diagnostics, ExitStatus,
    ResolvedConfig, Streams, run,
};
use crate::commands::{
    DbAdminCommand, DbAdminError, DbAdminOutcome, MigrationError, MigrationOptions, Operation,
};

#[derive(Debug, Clone, Copy)]
enum Reply {
    Completed,
    RetriesExhausted,
    IndexJobFailed,
    Unrecoverable,
}

struct Capture {
    command: DbAdminCommand,
    database_url: String,
    environment_url: Option<String>,
    pool_limit: i64,
}

struct FakeService {
    reply: Reply,
    log: Arc<Mutex<Vec<String>>>,
    captures: Arc<Mutex<Vec<Capture>>>,
}

impl DbAdminService for FakeService {
    fn run(&self, command: DbAdminCommand, config: ResolvedConfig) -> DbAdminFuture<'_> {
        Box::pin(async move {
            self.log.lock().expect("log").push("backend".to_owned());
            self.captures.lock().expect("captures").push(Capture {
                command,
                database_url: config.database_url.expose_secret().to_owned(),
                environment_url: config.environment.get("DATABASE_URL").cloned(),
                pool_limit: config.settings.sections.database_urls.pool_limit,
            });
            match self.reply {
                Reply::Completed => Ok(DbAdminOutcome::Completed),
                Reply::RetriesExhausted => Ok(DbAdminOutcome::RetriesExhausted),
                Reply::IndexJobFailed => Ok(DbAdminOutcome::IndexJobFailed),
                Reply::Unrecoverable => Err(DbAdminError::Unrecoverable {
                    source: MigrationError::Deploy(DeployError::Unfinished {
                        name: "20240101_init".to_owned(),
                    }),
                }),
            }
        })
    }
}

struct Harness {
    backend: DbAdminBackend,
    log: Arc<Mutex<Vec<String>>>,
    captures: Arc<Mutex<Vec<Capture>>>,
}

fn harness(reply: Reply) -> Harness {
    let log = Arc::new(Mutex::new(Vec::new()));
    let captures = Arc::new(Mutex::new(Vec::new()));
    let mut backend = DbAdminBackend::default();
    backend.services.insert(
        Operation::Migrate,
        Arc::new(FakeService {
            reply,
            log: Arc::clone(&log),
            captures: Arc::clone(&captures),
        }),
    );
    Harness {
        backend,
        log,
        captures,
    }
}

#[derive(Clone, Default)]
struct Recorder {
    lines: Arc<Mutex<Vec<String>>>,
}

impl Diagnostics for Recorder {
    fn output(&mut self, line: &str) {
        self.lines
            .lock()
            .expect("diagnostics")
            .push(format!("out|{line}"));
    }

    fn error(&mut self, line: &str) {
        self.lines
            .lock()
            .expect("diagnostics")
            .push(format!("err|{line}"));
    }
}

fn environment(pairs: &[(&str, &str)]) -> AdminEnvironment {
    AdminEnvironment {
        variables: pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
        home: None,
    }
}

fn pinned_environment() -> AdminEnvironment {
    environment(&[("DATABASE_URL", "postgresql://pinned.example/db")])
}

fn config_with_pool_limit(limit: i64) -> (TempDir, String) {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("config.yaml");
    fs::write(
        &path,
        format!("general_settings:\n  database_connection_pool_limit: {limit}\n"),
    )
    .expect("configuration file written");
    (directory, path.to_str().expect("utf-8 path").to_owned())
}

async fn invoke(
    arguments: &[&str],
    environment: AdminEnvironment,
    backend: &DbAdminBackend,
) -> (ExitStatus, Recorder) {
    let mut argv = vec![OsString::from("betterlite-db-admin")];
    argv.extend(arguments.iter().map(|argument| OsString::from(*argument)));
    let mut recorder = Recorder::default();
    let status = Box::pin(run(argv, environment, backend, &mut recorder)).await;
    (status, recorder)
}

fn lines_with(recorder: &Recorder, prefix: &str) -> Vec<String> {
    recorder
        .lines
        .lock()
        .expect("diagnostics")
        .iter()
        .filter_map(|line| line.strip_prefix(prefix).map(str::to_owned))
        .collect()
}

fn captured_command(harness: &Harness) -> DbAdminCommand {
    harness.captures.lock().expect("captures")[0]
        .command
        .clone()
}

fn captured(harness: &Harness) -> Vec<(String, Option<String>, i64)> {
    let captures = harness.captures.lock().expect("captures");
    captures
        .iter()
        .map(|capture| {
            (
                capture.database_url.clone(),
                capture.environment_url.clone(),
                capture.pool_limit,
            )
        })
        .collect()
}

#[tokio::test]
async fn help_lists_registered_operations() {
    let harness = harness(Reply::Completed);
    let (status, recorder) = invoke(&["--help"], environment(&[]), &harness.backend).await;
    assert_eq!(status, ExitStatus::Success);
    assert_eq!(status.code(), 0);
    assert!(
        lines_with(&recorder, "out|")
            .iter()
            .any(|line| line.contains("migrate"))
    );
    assert!(lines_with(&recorder, "err|").is_empty());
}

#[tokio::test]
async fn unknown_operation_fails() {
    let harness = harness(Reply::Completed);
    let (status, recorder) = invoke(&["frobnicate"], environment(&[]), &harness.backend).await;
    assert_eq!(status, ExitStatus::Usage);
    assert_eq!(status.code(), 2);
    assert!(lines_with(&recorder, "out|").is_empty());
    assert!(
        lines_with(&recorder, "err|")
            .iter()
            .any(|line| line.contains("frobnicate"))
    );
    assert!(harness.captures.lock().expect("captures").is_empty());
}

#[tokio::test]
async fn malformed_argv_exits_two_before_config() {
    let harness = harness(Reply::Completed);
    let (status, recorder) = invoke(
        &[
            "--config",
            "/nonexistent/betterlite.yaml",
            "migrate",
            "--bogus",
        ],
        environment(&[]),
        &harness.backend,
    )
    .await;
    assert_eq!(status, ExitStatus::Usage);
    let errors = lines_with(&recorder, "err|");
    assert!(errors.iter().any(|line| line.contains("--bogus")));
    assert!(!errors.iter().any(|line| line.contains("configuration")));
    assert!(harness.captures.lock().expect("captures").is_empty());
}

#[tokio::test]
async fn unregistered_backend_fails() {
    let backend = DbAdminBackend::default();
    let (status, recorder) = invoke(&["migrate"], pinned_environment(), &backend).await;
    assert_eq!(status, ExitStatus::Failure);
    assert!(lines_with(&recorder, "err|").iter().any(|line| {
        line.contains("no native service is registered for the migrate operation")
    }));
}

#[tokio::test]
async fn config_path_forwarded() {
    let harness = harness(Reply::Completed);
    let (_directory, path) = config_with_pool_limit(7);
    let before = invoke(
        &["--config", path.as_str(), "migrate"],
        pinned_environment(),
        &harness.backend,
    )
    .await;
    let after = invoke(
        &["migrate", "--config", path.as_str()],
        pinned_environment(),
        &harness.backend,
    )
    .await;
    assert_eq!(before.0, ExitStatus::Success);
    assert_eq!(after.0, ExitStatus::Success);
    let limits: Vec<i64> = captured(&harness)
        .iter()
        .map(|(_, _, pool_limit)| *pool_limit)
        .collect();
    assert_eq!(limits, vec![7, 7]);
}

#[tokio::test]
async fn pinned_url_not_replaced() {
    let harness = harness(Reply::Completed);
    let variables = environment(&[
        ("DATABASE_URL", "postgresql://pinned.example/db"),
        ("DATABASE_HOST", "assembled.example"),
        ("DATABASE_USER", "user"),
        ("DATABASE_NAME", "db"),
        ("DATABASE_PASSWORD", "secret"),
    ]);
    let (status, _) = invoke(&["migrate"], variables, &harness.backend).await;
    assert_eq!(status, ExitStatus::Success);
    let captures = captured(&harness);
    assert_eq!(captures[0].0, "postgresql://pinned.example/db");
    assert_eq!(
        captures[0].1.as_deref(),
        Some("postgresql://pinned.example/db")
    );
}

#[tokio::test]
async fn missing_url_exits_one_before_backend() {
    let harness = harness(Reply::Completed);
    let (status, recorder) = invoke(&["migrate"], environment(&[]), &harness.backend).await;
    assert_eq!(status, ExitStatus::Failure);
    assert_eq!(status.code(), 1);
    assert!(
        lines_with(&recorder, "err|")
            .iter()
            .any(|line| line.contains(
                "DATABASE_URL is not set and could not be assembled from the DATABASE_* env vars"
            ))
    );
    assert!(harness.captures.lock().expect("captures").is_empty());
}

#[tokio::test]
async fn url_resolution_error_stops_dispatch() {
    let harness = harness(Reply::Completed);
    let variables = environment(&[
        ("DATABASE_HOST", "[::1"),
        ("DATABASE_USER", "user"),
        ("DATABASE_NAME", "db"),
    ]);
    let (status, recorder) = invoke(&["migrate"], variables, &harness.backend).await;
    assert_eq!(status, ExitStatus::Failure);
    assert!(
        lines_with(&recorder, "err|")
            .iter()
            .any(|line| line.contains("Invalid IPv6 URL"))
    );
    assert!(harness.captures.lock().expect("captures").is_empty());
}

#[tokio::test]
async fn migration_flags_defaults() {
    let harness = harness(Reply::Completed);
    let (status, recorder) = invoke(&["migrate"], pinned_environment(), &harness.backend).await;
    assert_eq!(status, ExitStatus::Success);
    assert_eq!(
        captured_command(&harness),
        DbAdminCommand::Migrate(MigrationOptions {
            use_migrate: true,
            use_v2_resolver: true,
        })
    );
    assert!(lines_with(&recorder, "err|").iter().any(|line| {
        line.contains("Starting migration job (use_migrate=true, use_v2_resolver=true)")
    }));
}

#[rstest]
#[case::true_text("true", true)]
#[case::upper_true("TRUE", true)]
#[case::one("1", true)]
#[case::t("t", true)]
#[case::y("y", true)]
#[case::yes("yes", true)]
#[case::padded_true(" true ", false)]
#[case::on("on", false)]
#[case::false_text("false", false)]
#[case::empty("", false)]
#[case::no("no", false)]
#[tokio::test]
async fn migration_flags_case_and_whitespace(#[case] value: &str, #[case] expected: bool) {
    let harness = harness(Reply::Completed);
    let variables = environment(&[
        ("DATABASE_URL", "postgresql://pinned.example/db"),
        ("USE_V2_MIGRATION_RESOLVER", value),
    ]);
    let (status, _) = invoke(&["migrate"], variables, &harness.backend).await;
    assert_eq!(status, ExitStatus::Success);
    match captured_command(&harness) {
        DbAdminCommand::Migrate(options) => assert_eq!(options.use_v2_resolver, expected),
        DbAdminCommand::DatabaseViews => panic!("the migrate operation produced another command"),
        DbAdminCommand::DailyToolSpend(_)
        | DbAdminCommand::KeyTotalSpend
        | DbAdminCommand::KeyTotalSpendFromLogs
        | DbAdminCommand::SpendPartition(_) => {
            panic!("the migrate operation produced a spend command")
        }
        DbAdminCommand::UpdateUnassignedTeams | DbAdminCommand::MigrateVerificationTokens(_) => {
            panic!("the migrate operation produced a repair command")
        }
    }
}

#[rstest]
#[case::true_text("true", false)]
#[case::one("1", false)]
#[case::yes("yes", false)]
#[case::padded_false(" false ", true)]
#[case::false_text("false", true)]
#[case::empty("", true)]
#[tokio::test]
async fn push_inverts_use_migrate(#[case] value: &str, #[case] use_migrate: bool) {
    let harness = harness(Reply::Completed);
    let variables = environment(&[
        ("DATABASE_URL", "postgresql://pinned.example/db"),
        ("USE_PRISMA_DB_PUSH", value),
    ]);
    let (status, _) = invoke(&["migrate"], variables, &harness.backend).await;
    assert_eq!(status, ExitStatus::Success);
    match captured_command(&harness) {
        DbAdminCommand::Migrate(options) => assert_eq!(options.use_migrate, use_migrate),
        DbAdminCommand::DatabaseViews => panic!("the migrate operation produced another command"),
        DbAdminCommand::DailyToolSpend(_)
        | DbAdminCommand::KeyTotalSpend
        | DbAdminCommand::KeyTotalSpendFromLogs
        | DbAdminCommand::SpendPartition(_) => {
            panic!("the migrate operation produced a spend command")
        }
        DbAdminCommand::UpdateUnassignedTeams | DbAdminCommand::MigrateVerificationTokens(_) => {
            panic!("the migrate operation produced a repair command")
        }
    }
}

#[tokio::test]
async fn start_log_precedes_backend() {
    let harness = harness(Reply::Completed);
    let mut recorder = Recorder {
        lines: Arc::clone(&harness.log),
    };
    let argv = [
        OsString::from("betterlite-db-admin"),
        OsString::from("migrate"),
    ];
    let status = Box::pin(run(
        argv,
        pinned_environment(),
        &harness.backend,
        &mut recorder,
    ))
    .await;
    assert_eq!(status, ExitStatus::Success);
    let lines = harness.log.lock().expect("log").clone();
    let start = lines
        .iter()
        .position(|line| line.contains("Starting migration job"))
        .expect("start line");
    let backend = lines
        .iter()
        .position(|line| line == "backend")
        .expect("backend marker");
    assert!(start < backend);
}

#[rstest]
#[case::retries(Reply::RetriesExhausted)]
#[case::indexes(Reply::IndexJobFailed)]
#[tokio::test]
async fn retry_and_index_failure_exit_one(#[case] reply: Reply) {
    let harness = harness(reply);
    let (status, recorder) = invoke(&["migrate"], pinned_environment(), &harness.backend).await;
    assert_eq!(status, ExitStatus::Failure);
    assert_eq!(status.code(), 1);
    assert!(
        lines_with(&recorder, "err|")
            .iter()
            .any(|line| line.contains("Migration job failed after retries."))
    );
}

#[tokio::test]
async fn success_after_indexes_exit_zero() {
    let harness = harness(Reply::Completed);
    let (status, recorder) = invoke(&["migrate"], pinned_environment(), &harness.backend).await;
    assert_eq!(status, ExitStatus::Success);
    assert_eq!(status.code(), 0);
    assert!(
        lines_with(&recorder, "err|")
            .iter()
            .any(|line| line.contains("Migration job completed successfully."))
    );
}

#[tokio::test]
async fn unrecoverable_native_exit_two() {
    let harness = harness(Reply::Unrecoverable);
    let (status, recorder) = invoke(&["migrate"], pinned_environment(), &harness.backend).await;
    assert_eq!(status, ExitStatus::Unrecoverable);
    assert_eq!(status.code(), 2);
    assert!(lines_with(&recorder, "err|").iter().any(|line| {
        line.contains("Database migration cannot proceed: migration 20240101_init has an unfinished ledger row")
    }));
}

#[tokio::test]
async fn native_omits_client_generation() {
    let harness = harness(Reply::Completed);
    let (status, recorder) = invoke(&["migrate"], pinned_environment(), &harness.backend).await;
    assert_eq!(status, ExitStatus::Success);
    let errors = lines_with(&recorder, "err|");
    assert!(lines_with(&recorder, "out|").is_empty());
    assert!(!errors.iter().any(|line| line.contains("generate")));
    assert!(!errors.iter().any(|line| line.contains("prisma")));
    assert!(!errors.iter().any(|line| line.contains("stdout")));
    assert!(!errors.iter().any(|line| line.contains("stderr")));
    assert_eq!(errors.len(), 2);
    assert_eq!(harness.captures.lock().expect("captures").len(), 1);
}

#[tokio::test]
async fn assembled_url_stays_out_of_environment() {
    let harness = harness(Reply::Completed);
    let variables = environment(&[
        ("DATABASE_HOST", "assembled.example"),
        ("DATABASE_USER", "user"),
        ("DATABASE_NAME", "db"),
        ("DATABASE_PASSWORD", "secret"),
    ]);
    let (status, _) = invoke(&["migrate"], variables, &harness.backend).await;
    assert_eq!(status, ExitStatus::Success);
    let captures = captured(&harness);
    assert!(captures[0].0.contains("assembled.example"));
    assert_eq!(captures[0].1, None);
}

#[test]
fn streams_write_lines_and_success_is_zero() {
    let mut output = Vec::new();
    let mut errors = Vec::new();
    let mut streams = Streams {
        output: &mut output,
        errors: &mut errors,
    };
    streams.output("to stdout");
    streams.error("to stderr");
    assert_eq!(output, b"to stdout\n");
    assert_eq!(errors, b"to stderr\n");
    assert_eq!(ExitStatus::Success.code(), 0);
}

#[test]
fn non_utf8_variable_is_named_without_its_value() {
    let mut recorder = Recorder::default();
    let bad_value = OsString::from_vec(vec![0x66, 0xff]);
    let bad_name = OsString::from_vec(vec![0x6e, 0xff]);
    let loaded = AdminEnvironment::from_os(
        [
            (OsString::from("DATABASE_URL"), bad_value),
            (bad_name, OsString::from("kept-out")),
            (OsString::from("GOOD"), OsString::from("value")),
        ],
        None,
        &mut recorder,
    );
    assert_eq!(
        loaded
            .variables
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec!["GOOD"]
    );
    assert_eq!(
        lines_with(&recorder, "err|"),
        vec![
            "environment variable DATABASE_URL is not UTF-8 and is ignored".to_owned(),
            "an environment variable name is not UTF-8 and is ignored".to_owned(),
        ]
    );
}
