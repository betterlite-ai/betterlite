#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::database_startup::job::{
    JobOutcome, RequestLogIndexBuild, SetupFailure, classify, run_migration_job,
};
use betterlite_config::sections::database_startup::packaged;
use betterlite_config::sections::database_startup::startup::{
    StartupSettings, build_request_log_indexes, setup_database,
};
use betterlite_config::types::Config;
use betterlite_db::migration_drift::comparison::SchemaCheck;
use betterlite_db::migration_recovery::RecoveryError;
use betterlite_db::request_log_index_catalog::definitions::REQUEST_LOG_INDEXES;
use betterlite_db::worker_heartbeat::HostnameError;
use secrecy::SecretString;
use tempfile::NamedTempFile;
use tokio::sync::Notify;
use tokio::task::JoinSet;
use tracing::subscriber::with_default;

use self::postgres::{NoJitter, ledger_rows, migration, row_count, table_exists, with_schema};
use super::fixtures::{
    BuildAnswer, HeartbeatCall, JobBuild, MigrationAnswer, ScriptedHeartbeat, ScriptedRefresh,
    ScriptedStartup, heartbeat,
};
use super::heartbeat::HeartbeatTask;
use super::native::{NativeDatabase, NativeStartup, Running};
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

fn logged<T, F>(action: F) -> (T, String)
where
    F: FnOnce() -> T,
{
    let log = NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    let result = with_default(subscriber, action);
    (result, fs::read_to_string(log.path()).unwrap())
}

fn fallback_warnings(log: &str) -> usize {
    log.matches("falling back to").count()
}

fn pinned() -> StartupSettings {
    StartupSettings {
        updates_enabled: true,
        database_url: Some(SecretString::from(UNUSABLE_URL)),
        direct_url: None,
        use_v2_migration_resolver: true,
        use_prisma_db_push: false,
        set_replica_identity_full: false,
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
    let state = DatabaseState::from_settings(&settings);
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
    start_scripted(pinned(), scripted)
        .await
        .err()
        .expect("the startup policy fails")
}

#[tokio::test]
async fn schema_update_setting_precedence() {
    let matrix = [
        (
            "general_settings:\n  disable_prisma_schema_update: false\n",
            &[("DISABLE_SCHEMA_UPDATE", "true")][..],
            true,
        ),
        (
            "general_settings:\n  disable_prisma_schema_update: true\n",
            &[("DISABLE_SCHEMA_UPDATE", "false")][..],
            false,
        ),
        (
            "general_settings:\n  disable_prisma_schema_update: null\n",
            &[("DISABLE_SCHEMA_UPDATE", "yes")][..],
            false,
        ),
        (
            "general_settings:\n  disable_prisma_schema_update: \"TRUE\"\n",
            &[][..],
            false,
        ),
        (
            "general_settings: {}\n",
            &[("DISABLE_SCHEMA_UPDATE", "true")][..],
            false,
        ),
        ("general_settings: {}\n", &[][..], true),
    ];
    for (source, pairs, expected) in matrix {
        let mut settings = settings_from(source, pairs);
        assert_eq!(settings.updates_enabled, expected, "{source} {pairs:?}");
        settings.database_url = Some(SecretString::from(UNUSABLE_URL));
        let scripted = Arc::new(ScriptedStartup::new());
        let started = start_scripted(settings, Arc::clone(&scripted))
            .await
            .expect("the policy runs");
        if expected {
            assert_eq!(&started.outcome, &StartupOutcome::Migrated);
        } else {
            assert!(matches!(
                &started.outcome,
                StartupOutcome::UpdatesDisabled(_)
            ));
        }
        started.shutdown().await;
        let calls = if expected {
            vec!["migrate", "build"]
        } else {
            vec!["check"]
        };
        assert_eq!(scripted.calls(), calls, "{source} {pairs:?}");
    }
}

#[tokio::test]
async fn invalid_timeout_warning() {
    for invalid in ["abc", "0", "-5", "inf", "NaN"] {
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
        assert_eq!(fallback_warnings(&log), 3, "warnings for {invalid}");
        for name in [
            "LITELLM_PRISMA_COMMAND_TIMEOUT",
            "LITELLM_MIGRATION_LOCK_TIMEOUT",
            "LITELLM_PRISMA_MIGRATE_DEPLOY_TIMEOUT",
        ] {
            assert!(log.contains(name), "the warning names {name}: {log}");
        }
    }
}

#[tokio::test]
async fn deploy_timeout_precedence() {
    let inherited = settings_from(
        "general_settings: {}\n",
        &[("LITELLM_PRISMA_COMMAND_TIMEOUT", "900")],
    );
    assert_eq!(inherited.command_timeout, Duration::from_secs(900));
    assert_eq!(inherited.deploy_timeout, Duration::from_secs(900));

    let pinned_deploy = settings_from(
        "general_settings: {}\n",
        &[
            ("LITELLM_PRISMA_COMMAND_TIMEOUT", "900"),
            ("LITELLM_PRISMA_MIGRATE_DEPLOY_TIMEOUT", "1200"),
        ],
    );
    assert_eq!(pinned_deploy.deploy_timeout, Duration::from_secs(1200));

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
    assert_eq!(fallback_warnings(&log), 1);
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
    for name in [
        "PRISMA_OFFLINE_MODE",
        "PRISMA_CLI_PATH",
        "PRISMA_NODEENV_CACHE_DIR",
        "LITELLM_PRISMA_BOOTSTRAP_TIMEOUT",
        "LITELLM_MIGRATION_DIR",
    ] {
        assert!(log.contains(name), "the warning names {name}: {log}");
    }
}

#[tokio::test]
async fn native_prisma_bootstrap_variables_and_litellm_migration_dir_are_accepted_with() {
    let pairs = [
        ("DATABASE_URL", "postgresql://unreachable.example:1/db"),
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
    assert!(!settings.updates_enabled);
    // the obsolete inputs change no setting and start no Node toolchain: the schema check alone runs
    let startup = NativeStartup::packaged(settings.clone()).expect("the native startup builds");
    let state = DatabaseState::from_settings(&settings);
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
        updates_enabled: false,
        ..pinned()
    };
    let startup = NativeStartup::packaged(settings.clone()).expect("the native startup builds");
    let state = DatabaseState::from_settings(&settings);
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
        other => panic!("expected a warn-only outcome, got {other:?}"),
    }
    started.shutdown().await;
}

#[tokio::test]
async fn startup_migration_failure() {
    let scripted = Arc::new(ScriptedStartup {
        migrate: MigrationAnswer::Unverified,
        ..ScriptedStartup::new()
    });
    let error = start_scripted(pinned(), Arc::clone(&scripted))
        .await
        .err()
        .expect("a failed migration stops startup");
    match &error {
        StartupError::Unrecoverable { message } => {
            assert!(
                message.contains("could not be verified"),
                "the raised migration message is carried: {message}"
            );
        }
        other => panic!("expected an unrecoverable migration failure, got {other:?}"),
    }
    assert_eq!(error.exit_code(), 2);
    assert!(
        error
            .to_string()
            .starts_with("Database migration cannot proceed. ")
    );
    // a failed migration schedules no request-log index build
    assert_eq!(scripted.calls(), vec!["migrate"]);
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
        use_prisma_db_push: true,
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
    let error = setup_database(&no_url, &[], packaged::SCHEMA_PRISMA, &NoJitter)
        .await
        .expect_err("a migration without DATABASE_URL is refused");
    assert!(
        matches!(error, RecoveryError::DatabaseUrlRequired),
        "{error}"
    );
}

#[test]
fn state_not_configured_is_not_connected() {
    let settings = StartupSettings {
        database_url: None,
        direct_url: None,
        ..pinned()
    };
    let state = DatabaseState::from_settings(&settings);
    assert!(!state.is_configured());
    // proxy_server builds prisma_client only from DATABASE_URL, so a lone DIRECT_URL is no database
    let direct_only = DatabaseState::from_settings(&StartupSettings {
        database_url: None,
        direct_url: Some(SecretString::from(UNUSABLE_URL)),
        ..pinned()
    });
    assert!(!direct_only.is_configured());
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
    let state = DatabaseState::from_settings(&pinned());
    assert!(state.is_configured());
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
    assert_eq!(
        fake.calls(),
        vec![HeartbeatCall::Upsert, HeartbeatCall::Prune]
    );
    tokio::time::advance(Duration::from_secs(60)).await;
    tokio::task::yield_now().await;
    assert_eq!(
        fake.calls(),
        vec![
            HeartbeatCall::Upsert,
            HeartbeatCall::Prune,
            HeartbeatCall::Upsert,
            HeartbeatCall::Prune
        ]
    );
    task.stop().await.expect("the heartbeat task stops");
    assert_eq!(fake.calls().last(), Some(&HeartbeatCall::Deregister));
}

#[tokio::test]
async fn shutdown_drains() {
    let release = Arc::new(Notify::new());
    let finished = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let scripted = Arc::new(ScriptedStartup {
        build: BuildAnswer::WaitForRelease,
        release: Arc::clone(&release),
        finished: Arc::clone(&finished),
        ..ScriptedStartup::new()
    });
    let settings = pinned();
    let state = DatabaseState::from_settings(&settings);
    let started = start_scripted(settings, scripted)
        .await
        .expect("startup returns without the background build");
    assert!(!finished.load(Ordering::SeqCst));

    let refresh = Arc::new(ScriptedRefresh::default());
    let fake_heartbeat = Arc::new(ScriptedHeartbeat::default());
    let heartbeat_task =
        HeartbeatTask::start(Arc::new(heartbeat("worker-a")), Arc::clone(&fake_heartbeat)).await;
    let running = Running {
        state,
        startup: Some(started),
        heartbeat: Some(heartbeat_task),
        refresh: Some(refresh.clone()),
        pool: None,
    };
    let mut joins = JoinSet::new();
    joins.spawn(async move { running.shutdown().await });
    tokio::task::yield_now().await;
    release.notify_one();
    joins.join_next().await.expect("shutdown ends");
    assert!(
        finished.load(Ordering::SeqCst),
        "shutdown drained the index build instead of aborting it"
    );
    assert!(refresh.stopped.load(Ordering::SeqCst));
    assert_eq!(
        fake_heartbeat.calls().last(),
        Some(&HeartbeatCall::Deregister)
    );
}

#[tokio::test]
async fn native_database_reports_no_database_without_a_url() {
    let settings = StartupSettings {
        database_url: None,
        direct_url: None,
        ..pinned()
    };
    let scripted = Arc::new(ScriptedStartup::new());
    let database =
        NativeDatabase::<ScriptedStartup>::with_startup(Arc::clone(&scripted), settings, None);
    let running = database.start().await.expect("a database-less startup");
    assert_eq!(running.outcome(), Some(&StartupOutcome::NoDatabase));
    assert!(running.pool().is_none());
    running.shutdown().await;
    assert!(scripted.calls().is_empty());
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
        let started = Arc::clone(&build.started);
        let finished = Arc::clone(&build.finished);
        let release = Arc::clone(&build.release);
        let catalog = [migration(
            "20260101000000_b051_job",
            "CREATE TABLE \"B051_Job\" (id integer);",
        )];
        let mut tasks = JoinSet::new();
        let job_database = database.clone();
        tasks.spawn(async move {
            let settings = job_database.settings();
            let job_catalog = catalog.clone();
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
            "the job waits for the index build"
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
        assert!(finished.load(Ordering::SeqCst));
        assert!(table_exists(&database, "B051_Job").await);

        // a failed index build does not undo the applied migration; it reports the index exit
        let failing = JobBuild::immediate(false);
        let settings = database.settings();
        let outcome = run_migration_job(
            &settings,
            &catalog,
            packaged::SCHEMA_PRISMA,
            &NoJitter,
            &failing,
        )
        .await
        .expect("the job reports a failed build");
        assert_eq!(outcome, JobOutcome::IndexJobFailed);
        assert_eq!(ledger_rows(&database).await.len(), 1);
    })
    .await;
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
        let catalog = packaged::migrations();
        let rows = ledger_rows(&database).await;
        assert_eq!(rows.len(), catalog.len());
        for (row, pinned) in rows.iter().zip(catalog.iter()) {
            assert_eq!(row.name, pinned.name);
            assert_eq!(row.checksum, pinned.checksum);
            assert!(row.finished, "{} is finished", pinned.name);
            assert!(!row.rolled_back, "{} is not rolled back", pinned.name);
        }

        let client = database.pool.get().await.expect("a fixture connection");
        client
            .execute(
                "INSERT INTO \"LiteLLM_SpendLogs\" (\"request_id\", \"call_type\", \"startTime\", \"endTime\") \
                 VALUES ('b051-row', 'acompletion', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)",
                &[],
            )
            .await
            .expect("a pinned spend-log row inserts");
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
        for index in REQUEST_LOG_INDEXES {
            let name = index.name.to_owned();
            let exists: bool = client
                .query_one(
                    "SELECT EXISTS (SELECT 1 FROM pg_indexes WHERE schemaname = $1 AND indexname = $2)",
                    &[&database.schema, &name],
                )
                .await
                .expect("the index catalog reads")
                .get(0);
            assert!(exists, "the build created {name}");
        }

        let replica_identity: String = client
            .query_one(
                "SELECT c.relreplident::text FROM pg_class c \
                 JOIN pg_namespace n ON n.oid = c.relnamespace \
                 WHERE n.nspname = $1 AND c.relname = 'LiteLLM_SpendLogs'",
                &[&database.schema],
            )
            .await
            .expect("the replica identity reads")
            .get(0);
        assert_eq!(replica_identity, "d");
        let mut full_identity = settings;
        full_identity.set_replica_identity_full = true;
        setup_database(
            &full_identity,
            &packaged::migrations(),
            packaged::SCHEMA_PRISMA,
            &NoJitter,
        )
        .await
        .expect("the opt-in identity pass runs");
        let replica_identity: String = client
            .query_one(
                "SELECT c.relreplident::text FROM pg_class c \
                 JOIN pg_namespace n ON n.oid = c.relnamespace \
                 WHERE n.nspname = $1 AND c.relname = 'LiteLLM_SpendLogs'",
                &[&database.schema],
            )
            .await
            .expect("the replica identity reads")
            .get(0);
        assert_eq!(replica_identity, "f");
    })
    .await;
}

#[tokio::test]
async fn postgres_failure_effects() {
    with_schema(|database| async move {
        let settings = database.settings();
        let catalog = [
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
        ];
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

        let rows = ledger_rows(&database).await;
        assert_eq!(rows.len(), 2);
        assert!(rows[0].finished);
        assert!(!rows[0].rolled_back);
        assert!(!rows[1].finished);
        assert!(!rows[1].rolled_back);
        let logs = rows[1].logs.as_deref().expect("the failure is logged");
        assert!(logs.contains("B051_Missing"), "{logs}");

        // the unfinished row blocks the job like LiteLLM's P3018 until an operator resolves it
        let build = JobBuild::immediate(true);
        let error = run_migration_job(
            &settings,
            &catalog,
            packaged::SCHEMA_PRISMA,
            &NoJitter,
            &build,
        )
        .await
        .expect_err("the unfinished history stops the job");
        assert!(
            error.message.contains("could not be verified"),
            "{}",
            error.message
        );
        assert_eq!(ledger_rows(&database).await.len(), 2);
    })
    .await;
}

#[tokio::test]
async fn native_betterlite_db_admin_migrate_replaces_python_migrations_run_py_and_pris() {
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

        let client = database.pool.get().await.expect("a fixture connection");
        for index in REQUEST_LOG_INDEXES {
            let name = index.name.to_owned();
            let exists: bool = client
                .query_one(
                    "SELECT EXISTS (SELECT 1 FROM pg_indexes WHERE schemaname = $1 AND indexname = $2)",
                    &[&database.schema, &name],
                )
                .await
                .expect("the index catalog reads")
                .get(0);
            assert!(exists, "the migration job built {name}");
        }

        // a failed index build reports the index-job exit instead of reverting the migrations
        let failing = JobBuild::immediate(false);
        let outcome = run_migration_job(
            &settings,
            &packaged::migrations(),
            packaged::SCHEMA_PRISMA,
            &NoJitter,
            &failing,
        )
        .await
        .expect("an index failure is not a migration failure");
        assert_eq!(outcome, JobOutcome::IndexJobFailed);
    })
    .await;
}
