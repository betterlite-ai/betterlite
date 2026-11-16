#![cfg(test)]

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::error::Error as _;
use std::fs;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::database_startup::job::{
    JobOutcome, RequestLogIndexBuild, SetupFailure, classify, run_migration_job,
};
use betterlite_config::sections::database_startup::packaged;
use betterlite_config::sections::database_startup::startup::{
    StartupSettings, StartupSwitch, build_request_log_indexes, setup_database,
};
use betterlite_config::types::Config;
use betterlite_db::database_refresh::endpoint::DatabaseUrlSlot;
use betterlite_db::database_refresh::generation::GenerationOptions;
use betterlite_db::database_refresh::native::NativePoolProvider;
use betterlite_db::database_refresh_loop::clock::SystemClock;
use betterlite_db::database_refresh_loop::endpoint::WriterEndpoint;
use betterlite_db::database_refresh_loop::lifecycle::{DatabaseRefreshLoop, RefreshLoopConfig};
use betterlite_db::database_refresh_loop::minting::TokenMinter;
use betterlite_db::database_tokens::endpoint::IAMEndpoint;
use betterlite_db::database_tokens::expiry::TokenAuthKind;
use betterlite_db::migration_deploy::catalog::Migration;
use betterlite_db::migration_drift::comparison::SchemaCheck;
use betterlite_db::migration_recovery::RecoveryError;
use betterlite_db::worker_heartbeat::HostnameError;
use rstest::rstest;
use secrecy::SecretString;
use tempfile::NamedTempFile;
use tokio::task::{JoinSet, yield_now};
use tokio::time::{advance, timeout};
use tracing::subscriber::{DefaultGuard, set_default};

use self::postgres::{
    NoJitter, SchemaDatabase, catalog_request_log_indexes, ledger_rows, migration,
    present_request_log_indexes, row_count, scalar, table_exists, with_schema,
};
use super::fixtures::{
    BuildAnswer, HeartbeatCall, JobBuild, MigrationAnswer, ScriptedHeartbeat, ScriptedRefresh,
    ScriptedStartup, heartbeat,
};
use super::heartbeat::HeartbeatTask;
use super::native::{NativeDatabase, NativeStartup, Running};
use super::refresh::CredentialRefresh;
use super::serving::{ServingStartup, Startup, StartupError, StartupOutcome};
use super::state::{DatabaseState, DatabaseStatus};

mod postgres;

// A URL no connection can use: the disabled-update check reports Unavailable without a network round trip.
const UNUSABLE_URL: &str = "not a postgres connection string";

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn config(source: &str, pairs: &[(&str, &str)]) -> Config {
    ConfigLoader {
        lookup: environment(pairs),
    }
    .load(source)
    .expect("the config should load")
}

fn settings_from(source: &str, pairs: &[(&str, &str)]) -> StartupSettings {
    StartupSettings::from_config(&config(source, pairs))
}

fn capture_log() -> (NamedTempFile, DefaultGuard) {
    let log = NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    let guard = set_default(subscriber);
    (log, guard)
}

fn logged<T, F>(action: F) -> (T, String)
where
    F: FnOnce() -> T,
{
    let (log, guard) = capture_log();
    let result = action();
    drop(guard);
    (result, fs::read_to_string(log.path()).unwrap())
}

fn fallback_warnings(log: &str) -> usize {
    log.matches("falling back to").count()
}

fn pinned() -> StartupSettings {
    StartupSettings {
        updates_enabled: StartupSwitch(true),
        database_url: Some(SecretString::from(UNUSABLE_URL)),
        direct_url: None,
        use_v2_migration_resolver: StartupSwitch(true),
        use_prisma_db_push: StartupSwitch(false),
        set_replica_identity_full: StartupSwitch(false),
        command_timeout: Duration::from_secs(60),
        lock_timeout: Duration::from_secs(600),
        deploy_timeout: Duration::from_secs(600),
        bootstrap_inputs: Vec::new(),
    }
}

async fn start_scripted(
    settings: StartupSettings,
    startup: Arc<ScriptedStartup>,
) -> Result<Startup, StartupError> {
    let state = DatabaseState {
        configured: settings.database_url.is_some(),
    };
    ServingStartup {
        database: startup,
        settings: Arc::new(settings),
        state,
    }
    .start()
    .await
}

async fn policy_error(answer: MigrationAnswer) -> StartupError {
    let scripted = Arc::new(ScriptedStartup {
        migrate: answer,
        ..ScriptedStartup::new()
    });
    let Err(error) = start_scripted(pinned(), scripted).await else {
        panic!("the startup policy fails");
    };
    error
}

#[rstest]
#[case::setting_false_beats_enabled_environment(
    "general_settings:\n  disable_prisma_schema_update: false\n",
    &[("DISABLE_SCHEMA_UPDATE", "true")],
    true
)]
#[case::setting_true_beats_disabled_environment(
    "general_settings:\n  disable_prisma_schema_update: true\n",
    &[("DISABLE_SCHEMA_UPDATE", "false")],
    false
)]
#[case::explicit_null_reads_the_environment(
    "general_settings:\n  disable_prisma_schema_update: null\n",
    &[("DISABLE_SCHEMA_UPDATE", "yes")],
    false
)]
#[case::absent_reads_the_environment(
    "general_settings: {}\n",
    &[("DISABLE_SCHEMA_UPDATE", "true")],
    false
)]
#[case::nothing_set_enables_updates("general_settings: {}\n", &[], true)]
#[case::quoted_true_disables_updates(
    "general_settings:\n  disable_prisma_schema_update: \"TRUE\"\n",
    &[],
    false
)]
#[tokio::test]
async fn schema_update_setting_precedence(
    #[case] source: &str,
    #[case] pairs: &[(&str, &str)],
    #[case] updates_enabled: bool,
) {
    let mut settings = settings_from(source, pairs);
    assert_eq!(
        settings.updates_enabled.0, updates_enabled,
        "{source} {pairs:?}"
    );
    settings.database_url = Some(SecretString::from(UNUSABLE_URL));
    let scripted = Arc::new(ScriptedStartup::new());
    let started = start_scripted(settings, Arc::clone(&scripted))
        .await
        .expect("the policy runs");
    // the index build is a spawned task, so let it record its call before shutdown aborts it
    yield_now().await;
    if updates_enabled {
        assert_eq!(&started.outcome, &StartupOutcome::Migrated);
    } else {
        assert!(matches!(
            &started.outcome,
            StartupOutcome::UpdatesDisabled(_)
        ));
    }
    started.shutdown().await;
    let calls = if updates_enabled {
        vec!["migrate", "build"]
    } else {
        vec!["check"]
    };
    assert_eq!(
        scripted.calls.lock().expect("calls").clone(),
        calls,
        "{source} {pairs:?}"
    );
}

#[rstest]
#[case::text("abc")]
#[case::zero("0")]
#[case::negative("-5")]
#[case::infinite("inf")]
#[case::not_a_number("NaN")]
#[tokio::test]
async fn invalid_timeout_warning(#[case] invalid: &str) {
    let (settings, log) = logged(|| {
        settings_from(
            "general_settings: {}\n",
            &[
                ("LITELLM_PRISMA_COMMAND_TIMEOUT", invalid),
                ("LITELLM_MIGRATION_LOCK_TIMEOUT", invalid),
                ("LITELLM_PRISMA_MIGRATE_DEPLOY_TIMEOUT", invalid),
            ],
        )
    });
    assert_eq!(
        settings.command_timeout,
        Duration::from_secs(60),
        "{invalid}"
    );
    assert_eq!(settings.lock_timeout, Duration::from_secs(600), "{invalid}");
    assert_eq!(
        settings.deploy_timeout,
        Duration::from_secs(600),
        "{invalid}"
    );
    // the migration_ledger section reads the same three variables, so each unusable value warns twice
    assert_eq!(fallback_warnings(&log), 6, "warnings for {invalid}");
    assert!(log.contains("LITELLM_PRISMA_COMMAND_TIMEOUT"), "{log}");
    assert!(log.contains("LITELLM_MIGRATION_LOCK_TIMEOUT"), "{log}");
    assert!(
        log.contains("LITELLM_PRISMA_MIGRATE_DEPLOY_TIMEOUT"),
        "{log}"
    );
}

#[tokio::test]
async fn deploy_timeout_precedence() {
    let inherited = settings_from(
        "general_settings: {}\n",
        &[("LITELLM_PRISMA_COMMAND_TIMEOUT", "900")],
    );
    assert_eq!(inherited.command_timeout, Duration::from_mins(15));
    assert_eq!(inherited.deploy_timeout, Duration::from_mins(15));

    let pinned_deploy = settings_from(
        "general_settings: {}\n",
        &[
            ("LITELLM_PRISMA_COMMAND_TIMEOUT", "900"),
            ("LITELLM_PRISMA_MIGRATE_DEPLOY_TIMEOUT", "1200"),
        ],
    );
    assert_eq!(pinned_deploy.deploy_timeout, Duration::from_mins(20));

    let (invalid_deploy, log) = logged(|| {
        settings_from(
            "general_settings: {}\n",
            &[
                ("LITELLM_PRISMA_COMMAND_TIMEOUT", "900"),
                ("LITELLM_PRISMA_MIGRATE_DEPLOY_TIMEOUT", "soon"),
            ],
        )
    });
    // an unusable deploy value never inherits the command budget, unlike an absent one
    assert_eq!(invalid_deploy.deploy_timeout, Duration::from_secs(600));
    // the migration_ledger section reads the deploy variable too, so the warning appears twice
    assert_eq!(fallback_warnings(&log), 2);
}

#[tokio::test]
async fn obsolete_bootstrap_warning() {
    let pairs = [
        ("PRISMA_OFFLINE_MODE", "true"),
        ("PRISMA_CLI_PATH", "/opt/prisma"),
        ("PRISMA_NODEENV_CACHE_DIR", "/cache"),
        ("LITELLM_PRISMA_BOOTSTRAP_TIMEOUT", "900"),
        ("LITELLM_MIGRATION_DIR", "/migrations"),
    ];
    let (settings, log) = logged(|| settings_from("general_settings: {}\n", &pairs));
    assert_eq!(settings.bootstrap_inputs.len(), 5);
    assert_eq!(log.matches("accepted but have no effect").count(), 1);
    assert!(
        log.contains(
            "PRISMA_OFFLINE_MODE, PRISMA_CLI_PATH, PRISMA_NODEENV_CACHE_DIR, \
             LITELLM_PRISMA_BOOTSTRAP_TIMEOUT, LITELLM_MIGRATION_DIR"
        ),
        "the one warning names every accepted input: {log}"
    );
}

#[tokio::test]
async fn native_prisma_bootstrap_variables_and_litellm_migration_dir_are_accepted_with() {
    let pairs = [
        ("DATABASE_URL", "postgresql://unreachable.example:1/db"),
        ("DIRECT_URL", "postgresql://unreachable.example:1/direct"),
        ("PRISMA_OFFLINE_MODE", "true"),
        ("PRISMA_CLI_PATH", "/opt/prisma"),
        ("PRISMA_NODEENV_CACHE_DIR", "/cache"),
        ("LITELLM_PRISMA_BOOTSTRAP_TIMEOUT", "900"),
        ("LITELLM_MIGRATION_DIR", "/migrations"),
    ];
    let (settings, log) = logged(|| {
        settings_from(
            "general_settings:\n  disable_prisma_schema_update: true\n",
            &pairs,
        )
    });
    assert_eq!(settings.bootstrap_inputs.len(), 5);
    assert_eq!(log.matches("accepted but have no effect").count(), 1);
    assert!(!settings.updates_enabled.0);
    assert!(settings.direct_url.is_some());
    // the obsolete inputs change no setting and start no Node toolchain: the schema check alone runs
    let startup = NativeStartup::packaged(settings.clone()).expect("the native startup builds");
    let state = DatabaseState {
        configured: settings.database_url.is_some(),
    };
    let started = ServingStartup {
        database: Arc::new(startup),
        settings: Arc::new(settings),
        state,
    }
    .start()
    .await
    .expect("obsolete bootstrap inputs never fail startup");
    assert!(matches!(
        &started.outcome,
        StartupOutcome::UpdatesDisabled(_)
    ));
    started.shutdown().await;
}

#[tokio::test]
async fn disabled_updates_warn_only() {
    let settings = StartupSettings {
        updates_enabled: StartupSwitch(false),
        ..pinned()
    };
    let startup = NativeStartup::packaged(settings.clone()).expect("the native startup builds");
    let state = DatabaseState {
        configured: settings.database_url.is_some(),
    };
    let started = ServingStartup {
        database: Arc::new(startup),
        settings: Arc::new(settings),
        state,
    }
    .start()
    .await
    .expect("a schema comparison failure never stops startup");
    match &started.outcome {
        StartupOutcome::UpdatesDisabled(check) => assert_eq!(check, &SchemaCheck::Unavailable),
        other @ (StartupOutcome::NoDatabase | StartupOutcome::Migrated) => {
            panic!("expected a warn-only outcome, got {other:?}");
        }
    }
    started.shutdown().await;
}

#[tokio::test]
async fn startup_migration_failure() {
    let scripted = Arc::new(ScriptedStartup {
        migrate: MigrationAnswer::Unverified,
        ..ScriptedStartup::new()
    });
    let Err(error) = start_scripted(pinned(), Arc::clone(&scripted)).await else {
        panic!("a failed migration stops startup");
    };
    match &error {
        StartupError::Unrecoverable { source } => {
            assert!(
                matches!(source, RecoveryError::UnverifiedCompletion { .. }),
                "the raised migration error is carried: {source:?}"
            );
        }
        other @ (StartupError::SetupFailed
        | StartupError::Pool { .. }
        | StartupError::Heartbeat { .. }) => {
            panic!("expected an unrecoverable migration failure, got {other:?}");
        }
    }
    assert_eq!(error.exit_code(), 2);
    assert_eq!(
        error.to_string(),
        "Database migration cannot proceed. the migration completion could not be verified"
    );
    assert!(error.source().is_some());
    // a failed migration schedules no request-log index build
    assert_eq!(*scripted.calls.lock().expect("calls"), vec!["migrate"]);
}

#[tokio::test]
async fn migration_exit_codes() {
    let unrecoverable = policy_error(MigrationAnswer::Unverified).await;
    assert_eq!(unrecoverable.exit_code(), 2);

    let exhausted = policy_error(MigrationAnswer::Exhausted).await;
    assert_eq!(exhausted.exit_code(), 1);
    assert_eq!(
        exhausted.to_string(),
        "Database setup failed after multiple retries. The proxy cannot start safely. \
         Please check your database connection and migration status."
    );

    let heartbeat = StartupError::Heartbeat {
        source: HostnameError::Provider,
    };
    assert_eq!(heartbeat.exit_code(), 1);
}

#[tokio::test]
async fn db_push_is_refused_before_connecting() {
    let push = StartupSettings {
        use_prisma_db_push: StartupSwitch(true),
        ..pinned()
    };
    let error = setup_database(&push, &[], packaged::SCHEMA_PRISMA, &NoJitter)
        .await
        .expect_err("a db push request is refused");
    assert!(matches!(error, RecoveryError::DbPushRequested), "{error}");

    let no_url = StartupSettings {
        database_url: None,
        direct_url: None,
        ..pinned()
    };
    let missing_url = setup_database(&no_url, &[], packaged::SCHEMA_PRISMA, &NoJitter)
        .await
        .expect_err("a migration without DATABASE_URL is refused");
    assert!(
        matches!(missing_url, RecoveryError::DatabaseUrlRequired),
        "{missing_url}"
    );
}

#[test]
fn state_not_configured_is_not_connected() {
    let state = DatabaseState { configured: false };
    assert_eq!(
        state.public_status(DatabaseStatus::Connected),
        "Not connected"
    );
    assert_eq!(
        state.public_status(DatabaseStatus::Disconnected),
        "Not connected"
    );
}

#[test]
fn state_configured_reports_probe_status() {
    let state = DatabaseState { configured: true };
    assert_eq!(state.public_status(DatabaseStatus::Unknown), "unknown");
    assert_eq!(state.public_status(DatabaseStatus::Connected), "connected");
    assert_eq!(
        state.public_status(DatabaseStatus::Disconnected),
        "disconnected"
    );
}

#[tokio::test(start_paused = true)]
async fn heartbeat_beats_immediately_and_deregisters_on_stop() {
    let fake = Arc::new(ScriptedHeartbeat::default());
    let task = HeartbeatTask::start(Arc::new(heartbeat("worker-a")), Arc::clone(&fake)).await;
    yield_now().await;
    assert_eq!(
        *fake.calls.lock().expect("calls"),
        vec![HeartbeatCall::Upsert, HeartbeatCall::Prune]
    );
    advance(Duration::from_secs(60)).await;
    yield_now().await;
    assert_eq!(
        *fake.calls.lock().expect("calls"),
        vec![
            HeartbeatCall::Upsert,
            HeartbeatCall::Prune,
            HeartbeatCall::Upsert,
            HeartbeatCall::Prune
        ]
    );
    task.stop().await.expect("the heartbeat task stops");
    assert_eq!(
        fake.calls.lock().expect("calls").last(),
        Some(&HeartbeatCall::Deregister)
    );
}

#[tokio::test(start_paused = true)]
async fn shutdown_drains() {
    let (scripted, started, finished) = waiting_startup().await;
    assert_eq!(
        *scripted.calls.lock().expect("calls"),
        vec!["migrate", "build"]
    );
    assert!(!finished.load(Ordering::SeqCst));

    let refresh = Arc::new(ScriptedRefresh::default());
    let cloned_refresh = Arc::clone(&refresh);
    let erased_refresh: Arc<dyn CredentialRefresh> = cloned_refresh;
    let fake_heartbeat = Arc::new(ScriptedHeartbeat::default());
    let heartbeat_task =
        HeartbeatTask::start(Arc::new(heartbeat("worker-a")), Arc::clone(&fake_heartbeat)).await;
    yield_now().await;
    let running = Running {
        state: DatabaseState { configured: true },
        startup: Some(started),
        heartbeat: Some(heartbeat_task),
        refresh: Some(erased_refresh),
        pool: None,
    };
    let (log_file, guard) = capture_log();
    // the build is never released, so only an abort lets shutdown end before the paused clock's deadline
    timeout(Duration::from_hours(1), running.shutdown())
        .await
        .expect("shutdown aborts the index build instead of waiting for it");
    drop(guard);
    let log = fs::read_to_string(log_file.path()).unwrap();
    assert!(!log.contains("stopped with an error"), "{log}");
    assert!(!finished.load(Ordering::SeqCst));
    assert_eq!(
        Arc::strong_count(&scripted),
        1,
        "the aborted build task released its database handle"
    );
    assert!(refresh.stopped.load(Ordering::SeqCst));
    assert_eq!(
        fake_heartbeat.calls.lock().expect("calls").last(),
        Some(&HeartbeatCall::Deregister)
    );
}

async fn waiting_startup() -> (Arc<ScriptedStartup>, Startup, Arc<AtomicBool>) {
    let finished = Arc::new(AtomicBool::new(false));
    let scripted = Arc::new(ScriptedStartup {
        build: BuildAnswer::WaitForRelease,
        finished: Arc::clone(&finished),
        ..ScriptedStartup::new()
    });
    let started = start_scripted(pinned(), Arc::clone(&scripted))
        .await
        .expect("startup returns without the background build");
    yield_now().await;
    (scripted, started, finished)
}

#[rstest]
#[case::without_any_url(None)]
#[case::with_only_a_direct_url(Some(UNUSABLE_URL))]
#[tokio::test]
async fn native_database_reports_no_database_without_a_url(#[case] direct_url: Option<&str>) {
    // proxy_server builds prisma_client only from DATABASE_URL, so a lone DIRECT_URL is no database
    let settings = StartupSettings {
        database_url: None,
        direct_url: direct_url.map(SecretString::from),
        ..pinned()
    };
    let scripted = Arc::new(ScriptedStartup::new());
    let database =
        NativeDatabase::<ScriptedStartup>::with_startup(Arc::clone(&scripted), settings, None);
    let running = database.start().await.expect("a database-less startup");
    assert!(!running.state.configured);
    assert_eq!(
        running.startup.as_ref().map(|startup| &startup.outcome),
        Some(&StartupOutcome::NoDatabase)
    );
    assert!(running.pool.is_none());
    running.shutdown().await;
    assert!(scripted.calls.lock().expect("calls").is_empty());
}

#[tokio::test]
async fn native_database_with_a_url_is_configured() {
    let scripted = Arc::new(ScriptedStartup::new());
    let database =
        NativeDatabase::<ScriptedStartup>::with_startup(Arc::clone(&scripted), pinned(), None);
    let Err(error) = database.start().await else {
        panic!("an unusable DATABASE_URL cannot open the pool");
    };
    assert!(matches!(error, StartupError::Pool { .. }), "{error}");
    assert_eq!(error.exit_code(), 1);
    // the configured state ran the migration before the pool failed
    assert_eq!(
        scripted.calls.lock().expect("calls").first(),
        Some(&"migrate")
    );
}

#[tokio::test]
async fn existing_schema_no_rewrite() {
    with_schema(|database| async move {
        let settings = database.settings();
        let catalog = [migration(
            "20260101000000_b051_existing",
            "CREATE TABLE \"B051_Existing\" (id integer); \
             INSERT INTO \"B051_Existing\" (id) VALUES (7);",
        )];
        setup_database(&settings, &catalog, packaged::SCHEMA_PRISMA, &NoJitter)
            .await
            .expect("the first startup applies the migration");
        let first = ledger_rows(&database).await;
        assert_eq!(first.len(), 1);
        assert!(first[0].finished);
        assert_eq!(row_count(&database, "B051_Existing").await, 1);

        // a startup against a schema that is already current applies nothing and rewrites nothing
        setup_database(&settings, &catalog, packaged::SCHEMA_PRISMA, &NoJitter)
            .await
            .expect("the second startup finds the schema current");
        let second = ledger_rows(&database).await;
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].checksum, first[0].checksum);
        assert!(second[0].finished);
        assert!(!second[0].rolled_back);
        assert_eq!(row_count(&database, "B051_Existing").await, 1);
    })
    .await;
}

#[tokio::test]
async fn migration_job_waits() {
    with_schema(|database| async move {
        let build = JobBuild::waiting();
        let started = Arc::clone(&build.signals.started);
        let finished = Arc::clone(&build.signals.finished);
        let release = Arc::clone(&build.signals.release);
        let catalog = [migration(
            "20260101000000_b051_job",
            "CREATE TABLE \"B051_Job\" (id integer);",
        )];
        let job_catalog = catalog.clone();
        let settings = database.settings();
        let mut tasks = JoinSet::new();
        tasks.spawn(async move {
            run_migration_job(
                &settings,
                &job_catalog,
                packaged::SCHEMA_PRISMA,
                &NoJitter,
                &build,
            )
            .await
        });
        started.notified().await;
        assert!(
            !finished.load(Ordering::SeqCst),
            "the job waits for the build"
        );
        assert!(tasks.try_join_next().is_none(), "the job is still running");
        release.notify_one();
        let outcome = tasks
            .join_next()
            .await
            .expect("the job task")
            .expect("the job joins")
            .expect("the job completes");
        assert_eq!(outcome, JobOutcome::Completed);
        assert_failed_index_build_keeps_the_migration(&database, &catalog).await;
    })
    .await;
}

// a failed index build does not undo the applied migration; it reports the index exit
async fn assert_failed_index_build_keeps_the_migration(
    database: &SchemaDatabase,
    catalog: &[Migration],
) {
    let failing = JobBuild::immediate(false);
    let settings = database.settings();
    let outcome = run_migration_job(
        &settings,
        catalog,
        packaged::SCHEMA_PRISMA,
        &NoJitter,
        &failing,
    )
    .await
    .expect("the job reports a failed build");
    assert_eq!(outcome, JobOutcome::IndexJobFailed);
    assert!(table_exists(database, "B051_Job").await);
    assert_eq!(ledger_rows(database).await.len(), 1);
}

#[tokio::test]
async fn postgres_pinned_rows() {
    with_schema(|database| async move {
        let settings = database.settings();
        setup_database(
            &settings,
            &packaged::migrations(),
            packaged::SCHEMA_PRISMA,
            &NoJitter,
        )
        .await
        .expect("the packaged migrations apply");
        assert_packaged_ledger(&database).await;

        insert_spend_log(&database).await;
        setup_database(
            &settings,
            &packaged::migrations(),
            packaged::SCHEMA_PRISMA,
            &NoJitter,
        )
        .await
        .expect("a second startup keeps the schema");
        assert_eq!(row_count(&database, "LiteLLM_SpendLogs").await, 1);
        assert!(build_request_log_indexes(&settings).await);
        assert_eq!(
            present_request_log_indexes(&database).await,
            catalog_request_log_indexes(),
            "the build created every request-log index"
        );
        assert_eq!(spend_log_replica_identity(&database).await, "d");
        assert_replica_identity_full(&database, &settings).await;
    })
    .await;
}

async fn assert_packaged_ledger(database: &SchemaDatabase) {
    let catalog = packaged::migrations();
    assert!(
        catalog
            .iter()
            .any(|migration| migration.name == "20250326162113_baseline"),
        "the packaged catalog carries the baseline migration"
    );
    let rows = ledger_rows(database).await;
    let ledger: Vec<(String, String, bool, bool)> = rows
        .iter()
        .map(|row| {
            (
                row.name.clone(),
                row.checksum.clone(),
                row.finished,
                row.rolled_back,
            )
        })
        .collect();
    let pinned: Vec<(String, String, bool, bool)> = catalog
        .iter()
        .map(|migration| {
            (
                migration.name.clone(),
                migration.checksum.clone(),
                true,
                false,
            )
        })
        .collect();
    assert_eq!(
        ledger, pinned,
        "the ledger records every packaged migration"
    );
}

async fn insert_spend_log(database: &SchemaDatabase) {
    let client = database.pool.get().await.expect("a fixture connection");
    let inserted = client
        .execute(
            "INSERT INTO \"LiteLLM_SpendLogs\" (\"request_id\", \"call_type\", \"startTime\", \"endTime\") \
             VALUES ('b051-row', 'acompletion', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)",
            &[],
        )
        .await
        .expect("a pinned spend-log row inserts");
    assert_eq!(inserted, 1);
}

async fn spend_log_replica_identity(database: &SchemaDatabase) -> String {
    scalar(
        database,
        "SELECT c.relreplident::text FROM pg_class c \
         JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = $1 AND c.relname = 'LiteLLM_SpendLogs'",
        &[&database.schema],
    )
    .await
}

async fn assert_replica_identity_full(database: &SchemaDatabase, settings: &StartupSettings) {
    let mut full_identity = settings.clone();
    full_identity.set_replica_identity_full = StartupSwitch(true);
    setup_database(
        &full_identity,
        &packaged::migrations(),
        packaged::SCHEMA_PRISMA,
        &NoJitter,
    )
    .await
    .expect("the opt-in identity pass runs");
    assert_eq!(spend_log_replica_identity(database).await, "f");
}

#[tokio::test]
async fn postgres_failure_effects() {
    with_schema(|database| async move {
        let settings = database.settings();
        let catalog = failing_catalog();
        let recovery = setup_database(&settings, &catalog, packaged::SCHEMA_PRISMA, &NoJitter)
            .await
            .expect_err("the failing migration stops setup");
        assert_eq!(classify(&recovery), SetupFailure::Unrecoverable);
        assert!(
            recovery.to_string().contains("20260101000002_b051_second"),
            "the failure names its migration: {recovery}"
        );

        // the preceding migration committed, and the failing migration's transaction left nothing
        assert!(table_exists(&database, "B051_First").await);
        assert_eq!(row_count(&database, "B051_First").await, 1);
        assert!(!table_exists(&database, "B051_Second").await);
        assert_failed_ledger_row(&database).await;
        assert_unfinished_history_stops_the_job(&database, &catalog).await;
    })
    .await;
}

fn failing_catalog() -> [Migration; 2] {
    [
        migration(
            "20260101000001_b051_first",
            "CREATE TABLE \"B051_First\" (id integer); \
             INSERT INTO \"B051_First\" (id) VALUES (1);",
        ),
        migration(
            "20260101000002_b051_second",
            "CREATE TABLE \"B051_Second\" (id integer); \
             INSERT INTO \"B051_Missing\" (id) VALUES (1);",
        ),
    ]
}

async fn assert_failed_ledger_row(database: &SchemaDatabase) {
    let rows = ledger_rows(database).await;
    assert_eq!(rows.len(), 2);
    assert!(rows[0].finished);
    assert!(!rows[0].rolled_back);
    assert!(!rows[1].finished);
    assert!(!rows[1].rolled_back);
    let logs = rows[1].logs.as_deref().expect("the failure is logged");
    assert!(logs.contains("B051_Missing"), "{logs}");
}

// the unfinished row blocks the job like LiteLLM's P3018 until an operator resolves it
async fn assert_unfinished_history_stops_the_job(database: &SchemaDatabase, catalog: &[Migration]) {
    let build = JobBuild::immediate(true);
    let settings = database.settings();
    let error = run_migration_job(
        &settings,
        catalog,
        packaged::SCHEMA_PRISMA,
        &NoJitter,
        &build,
    )
    .await
    .expect_err("the unfinished history stops the job");
    assert_eq!(classify(&error), SetupFailure::Unrecoverable);
    assert!(
        error.to_string().contains("could not be verified"),
        "{error}"
    );
    assert_eq!(ledger_rows(database).await.len(), 2);
}

#[tokio::test]
async fn native_betterlite_db_admin_migrate_replaces_python_migrations_run_py_and_prisma_migration()
{
    with_schema(|database| async move {
        let mut settings = database.settings();
        settings.bootstrap_inputs = vec!["PRISMA_OFFLINE_MODE", "LITELLM_MIGRATION_DIR"];
        let build = RequestLogIndexBuild {
            settings: &settings,
        };
        let outcome = run_migration_job(
            &settings,
            &packaged::migrations(),
            packaged::SCHEMA_PRISMA,
            &NoJitter,
            &build,
        )
        .await
        .expect("the native migration job completes");
        assert_eq!(outcome, JobOutcome::Completed);
        let rows = ledger_rows(&database).await;
        assert_eq!(rows.len(), packaged::migrations().len());
        assert!(rows.iter().all(|row| row.finished && !row.rolled_back));

        assert_eq!(
            present_request_log_indexes(&database).await,
            catalog_request_log_indexes(),
            "the migration job built every request-log index"
        );

        // a failed index build reports the index-job exit instead of reverting the migrations
        let failing = JobBuild::immediate(false);
        let index_failure = run_migration_job(
            &settings,
            &packaged::migrations(),
            packaged::SCHEMA_PRISMA,
            &NoJitter,
            &failing,
        )
        .await
        .expect("an index failure is not a migration failure");
        assert_eq!(index_failure, JobOutcome::IndexJobFailed);
    })
    .await;
}

// The refresh task only mints when the loop serves a token-authenticated pool; nothing here reaches a database.
struct NoMint;

impl TokenMinter for NoMint {
    type Error = Infallible;

    fn token(&self, _endpoint: &IAMEndpoint) -> Result<Option<SecretString>, Infallible> {
        Ok(None)
    }
}

#[tokio::test]
async fn credential_refresh_starts_and_stops_the_token_refresh_task() {
    let options = GenerationOptions {
        iam_token_db_auth: false,
        token_auth: Some(TokenAuthKind::Rds),
        db_url_env_var: "DATABASE_URL".to_owned(),
        url: DatabaseUrlSlot::default(),
        iam_endpoint: None,
        recreate_uses_datasource: false,
        log_prefix: "[test]".to_owned(),
    };
    let refresh = DatabaseRefreshLoop::<NativePoolProvider, NoMint, Infallible>::new(
        Arc::new(NativePoolProvider),
        options,
        None,
        RefreshLoopConfig {
            writer_endpoint: WriterEndpoint {
                host: None,
                port: "5432".to_owned(),
                user: None,
                name: None,
                schema: None,
            },
            minter: NoMint,
            clock: Arc::new(SystemClock),
        },
        None,
    )
    .expect("the refresh loop builds");
    CredentialRefresh::start(&refresh);
    assert!(refresh.has_task(), "start spawns the token refresh task");
    CredentialRefresh::stop(&refresh)
        .await
        .expect("the refresh task stops");
    assert!(!refresh.has_task(), "stop clears the refresh task");
}
