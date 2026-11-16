#![cfg(test)]

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::error::Error as _;
use std::fs;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::database_startup::startup::{StartupSettings, StartupSwitch};
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
use betterlite_db::migration_drift::comparison::SchemaCheck;
use betterlite_db::migration_recovery::RecoveryError;
use betterlite_db::worker_heartbeat::HostnameError;
use rstest::rstest;
use secrecy::SecretString;
use tempfile::NamedTempFile;
use tokio::task::yield_now;
use tokio::time::{advance, timeout};
use tracing::subscriber::{DefaultGuard, set_default};

use super::fixtures::{
    BuildAnswer, HeartbeatCall, MigrationAnswer, ScriptedHeartbeat, ScriptedRefresh,
    ScriptedStartup, heartbeat,
};
use super::heartbeat::HeartbeatTask;
use super::native::{NativeDatabase, NativeStartup, Running};
use super::refresh::CredentialRefresh;
use super::serving::{ServingStartup, Startup, StartupError, StartupOutcome};
use super::state::{DatabaseState, DatabaseStatus};

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
