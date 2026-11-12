use std::future::ready;
use std::slice::from_ref;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::net::TcpListener;

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tracing::subscriber::set_default;

use crate::migration_deploy::catalog::Migration;
use crate::migration_recovery::outcome::{DeployOutcome, FailedIdentity, FailedStep, StepSource};
use crate::migration_recovery::policy::{PassSettings, RecoverySteps as _, run_passes};
use crate::migration_recovery::steps::{PgSteps, PgStepsInputs, recoverable_name};
use crate::migration_recovery::tests::database::{
    COMMAND_TIMEOUT, LOCK_TIMEOUT, LedgerRow, create_ledger, insert_row, migration, next_id,
    raised_database_error, relation_exists, stored_row, with_schema,
};
use crate::migration_recovery::tests::support::{
    FakeJitter, RecordingPause, ScriptedSteps, broken_cause, capture_logs,
};
use crate::migration_recovery::{
    RecoveryError, SetupInputs, SetupRequest, connection_urls, ledger_namespace,
    replica_identity_if_requested, resolve_v2_migration_resolver, setup_database_v2,
};

fn settings(deploy_seconds: u64, lock_seconds: u64) -> PassSettings {
    PassSettings {
        deploy_timeout: Duration::from_secs(deploy_seconds),
        lock_timeout: Duration::from_secs(lock_seconds),
    }
}

fn request() -> SetupRequest {
    SetupRequest {
        use_v2_resolver: true,
        db_push_requested: false,
    }
}

fn inputs<'input>(
    database_url: Option<&str>,
    direct_url: Option<&str>,
    catalog: &'input [Migration],
    jitter: &'input FakeJitter,
) -> SetupInputs<'input> {
    SetupInputs {
        database_url: database_url.map(SecretString::from),
        direct_url: direct_url.map(SecretString::from),
        catalog,
        schema_prisma: &[],
        current_build: None,
        command_timeout: COMMAND_TIMEOUT,
        deploy_timeout: Duration::from_secs(42),
        lock_timeout: LOCK_TIMEOUT,
        jitter,
        next_id,
    }
}

#[tokio::test]
async fn writer_url_required_even_with_direct() {
    let jitter = FakeJitter(0);
    let direct = "postgresql://user:pass@direct:5432/db";
    let absent = inputs(None, Some(direct), &[], &jitter);
    let error = setup_database_v2(&absent, request()).await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "DATABASE_URL is required for v2 migrations"
    );
    let empty = inputs(Some(""), Some(direct), &[], &jitter);
    let empty_error = setup_database_v2(&empty, request()).await.unwrap_err();
    assert_eq!(
        empty_error.to_string(),
        "DATABASE_URL is required for v2 migrations"
    );
}

#[test]
fn direct_lock_url_writer_namespace() {
    let writer = SecretString::from(
        "postgresql://u:p@writer:5432/db?schema=tenant&connection_limit=5&pgbouncer=true",
    );
    let direct =
        SecretString::from("postgresql://u:p@direct:5432/db?sslmode=require&connection_limit=7");
    let urls = connection_urls(&writer, Some(&direct)).expect("native URLs");
    let lock = urls.lock.expose_secret();
    assert!(
        lock.starts_with("postgresql://u:p@direct:5432/db"),
        "{lock}"
    );
    assert!(!lock.contains("connection_limit"), "{lock}");
    assert!(!lock.contains("schema"), "{lock}");
    assert!(lock.contains("sslmode=require"), "{lock}");
    let deploy = urls.deploy.expose_secret();
    assert!(
        deploy.starts_with("postgresql://u:p@direct:5432/db"),
        "{deploy}"
    );
    assert!(!deploy.contains("schema"), "{deploy}");
    let empty_direct = SecretString::from("");
    let writer_only = connection_urls(&writer, Some(&empty_direct)).expect("native URLs");
    let fallback = writer_only.lock.expose_secret();
    assert!(
        fallback.starts_with("postgresql://u:p@writer:5432/db"),
        "{fallback}"
    );
    assert_eq!(
        ledger_namespace(writer.expose_secret()).expect("a namespace"),
        "tenant"
    );
    assert_eq!(
        ledger_namespace("postgresql://u:p@writer:5432/db").expect("a namespace"),
        "public"
    );
    assert_eq!(
        ledger_namespace("postgresql://u:p@writer:5432/db?schema=").expect("a namespace"),
        "public"
    );
}

#[rstest]
#[case::traversal("../20240101000000_known")]
#[case::backslash("a\\b")]
#[case::dot(".")]
#[case::dotdot("..")]
#[case::empty("")]
#[case::unknown("unknown")]
#[tokio::test]
async fn recovery_rejects_path_and_unknown_name(#[case] rejected: &str) {
    let catalog = [migration(
        "20240101000000_known",
        "CREATE TABLE known_probe (id integer)",
    )];
    let jitter = FakeJitter(0);
    let steps = PgSteps::new(PgStepsInputs {
        schema: "public".to_owned(),
        catalog: &catalog,
        prisma_url: SecretString::from("postgresql://user:pass@127.0.0.1:1/db"),
        deploy_url: SecretString::from("postgresql://user:pass@127.0.0.1:1/db"),
        lock_url: SecretString::from("not a database url"),
        database_url: SecretString::from("postgresql://user:pass@127.0.0.1:1/db"),
        direct_url: None,
        schema_prisma: &[],
        current_build: None,
        command_timeout: COMMAND_TIMEOUT,
        lock_timeout: LOCK_TIMEOUT,
        jitter: &jitter,
        next_id,
    })
    .expect("an offline deploy pool builds");
    assert!(
        !steps
            .recover_completed(rejected)
            .await
            .expect("a rejected name never opens a coordinator")
    );
    let error = steps
        .recover_completed("20240101000000_known")
        .await
        .expect_err("a catalog name reaches the coordinator connection");
    assert!(matches!(error, RecoveryError::Connection { .. }), "{error}");
}

#[rstest]
#[case::plain("plain", true)]
#[case::slash("a/b", false)]
#[case::backslash("a\\b", false)]
#[case::dot(".", false)]
#[case::dotdot("..", false)]
#[case::empty("", false)]
fn recoverable_name_rejects_hostile_names(#[case] name: &str, #[case] recoverable: bool) {
    assert_eq!(recoverable_name(name), recoverable);
}

#[tokio::test]
async fn recovery_query_timeout_is_typed() {
    // A listener that never answers keeps the connection handshake pending past the command timeout.
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a listener");
    let address = listener.local_addr().expect("the bound address");
    let url = format!("postgresql://user:pass@{address}/db");
    let catalog = [migration(
        "20240101000000_known",
        "CREATE TABLE known_probe (id integer)",
    )];
    let jitter = FakeJitter(0);
    let steps = PgSteps::new(PgStepsInputs {
        schema: "public".to_owned(),
        catalog: &catalog,
        prisma_url: SecretString::from(url.clone()),
        deploy_url: SecretString::from(url.clone()),
        lock_url: SecretString::from(url.clone()),
        database_url: SecretString::from(url),
        direct_url: None,
        schema_prisma: &[],
        current_build: None,
        command_timeout: Duration::from_millis(50),
        lock_timeout: LOCK_TIMEOUT,
        jitter: &jitter,
        next_id,
    })
    .expect("an offline deploy pool builds");
    let identity = steps
        .unfinished_identity("20240101000000_known")
        .await
        .expect_err("an identity read past its command timeout is a typed error");
    assert!(matches!(
        identity,
        RecoveryError::QueryTimeout { seconds: 0 }
    ));
    let lock = steps
        .deploy_lock_held()
        .await
        .expect_err("a lock read past its command timeout is a typed error");
    assert!(matches!(lock, RecoveryError::QueryTimeout { seconds: 0 }));
}

#[tokio::test]
async fn baseline_connection_failure_propagates() {
    let catalog = [migration(
        "20240101000000_known",
        "CREATE TABLE known_probe (id integer)",
    )];
    let jitter = FakeJitter(0);
    let steps = PgSteps::new(PgStepsInputs {
        schema: "public".to_owned(),
        catalog: &catalog,
        prisma_url: SecretString::from("postgresql://user:pass@127.0.0.1:1/db"),
        deploy_url: SecretString::from("postgresql://user:pass@127.0.0.1:1/db"),
        lock_url: SecretString::from("not a database url"),
        database_url: SecretString::from("postgresql://user:pass@127.0.0.1:1/db"),
        direct_url: None,
        schema_prisma: &[],
        current_build: None,
        command_timeout: COMMAND_TIMEOUT,
        lock_timeout: LOCK_TIMEOUT,
        jitter: &jitter,
        next_id,
    })
    .expect("an offline deploy pool builds");
    let error = steps
        .baseline()
        .await
        .expect_err("a baseline failure stops startup");
    assert!(matches!(error, RecoveryError::Connection { .. }), "{error}");
}

#[tokio::test]
async fn completion_before_inert_rollback() {
    with_schema(|database| async move {
        create_ledger(&database).await;
        let migration = migration("20240101000000_inert", "SELECT 1;");
        insert_row(
            &database,
            &LedgerRow {
                id: "00000000-0000-0000-0000-000000000001",
                name: &migration.name,
                checksum: &migration.checksum,
                steps: 1,
                logs: Some(""),
                finished: false,
                rolled_back: false,
                started_at: "2024-01-01 00:00:00+00",
            },
        )
        .await;
        let jitter = FakeJitter(0);
        let steps = database.steps(from_ref(&migration), &jitter);
        assert!(
            steps
                .recover_completed(&migration.name)
                .await
                .expect("recovery")
        );
        let row = stored_row(&database, &migration.name).await;
        assert!(
            row.finished,
            "confirmed completion wins over the inert rollback"
        );
        assert!(!row.rolled_back);
        assert_eq!(row.steps, 1_i32);
        assert_eq!(row.checksum, migration.checksum);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn baseline_checkpoint_before_new_pass() {
    let steps = ScriptedSteps::default();
    steps.deploying(DeployOutcome::SchemaNotBaselined);
    steps.deploying(DeployOutcome::Complete);
    let jitter = FakeJitter(0);
    let pause = RecordingPause::default();
    run_passes(&steps, settings(42, 77), &jitter, &pause)
        .await
        .expect("a baseline checkpoint restarts the pass");
    assert_eq!(
        steps.events(),
        vec!["ahead", "deploy", "baseline", "ahead", "deploy"]
    );
    assert!(pause.delays().is_empty());
}

#[tokio::test(start_paused = true)]
async fn recovery_checkpoint_resets_budgets() {
    let steps = ScriptedSteps::default();
    let identity = FailedIdentity {
        name: "20240101000000_a".to_owned(),
        started_at: "2024-01-01 00:00:00+00".to_owned(),
    };
    (0_u32..3).for_each(|_| steps.deploying(DeployOutcome::TimedOut));
    steps.recovering(Ok(true));
    steps.deploying(DeployOutcome::UnfinishedHistory {
        identity: Some(identity),
    });
    (0_u32..3).for_each(|_| steps.deploying(DeployOutcome::TimedOut));
    steps.deploying(DeployOutcome::Complete);
    let jitter = FakeJitter(0);
    let pause = RecordingPause::default();
    run_passes(&steps, settings(42, 77), &jitter, &pause)
        .await
        .expect("each checkpoint restarts the four attempts");
    assert_eq!(steps.deploy_timeouts().len(), 8);
    assert_eq!(pause.delays().len(), 6);
}

#[tokio::test(start_paused = true)]
async fn callback_failure_propagates() {
    let identity = FailedIdentity {
        name: "20240101000000_a".to_owned(),
        started_at: "2024-01-01 00:00:00+00".to_owned(),
    };
    let steps = ScriptedSteps::default();
    steps.deploying(DeployOutcome::SchemaNotBaselined);
    steps.baselining(Err(RecoveryError::ManualIntervention {
        message: "baseline broke".to_owned(),
        source: broken_cause(),
    }));
    let jitter = FakeJitter(0);
    let pause = RecordingPause::default();
    let error = run_passes(&steps, settings(42, 77), &jitter, &pause)
        .await
        .expect_err("a baseline failure stops startup");
    assert!(error.to_string().contains("baseline broke"), "{error}");
    assert_eq!(steps.events(), vec!["ahead", "deploy", "baseline"]);
    let recovery = ScriptedSteps::default();
    recovery.deploying(DeployOutcome::UnfinishedHistory {
        identity: Some(identity),
    });
    recovery.recovering(Err(RecoveryError::ManualIntervention {
        message: "recovery broke".to_owned(),
        source: broken_cause(),
    }));
    let recovery_error = run_passes(&recovery, settings(42, 77), &jitter, &pause)
        .await
        .expect_err("a recovery failure stops startup");
    assert!(
        recovery_error.to_string().contains("recovery broke"),
        "{recovery_error}"
    );
}

#[tokio::test(start_paused = true)]
async fn ahead_history_warning_is_nonfatal() {
    let steps = ScriptedSteps::default();
    steps.report_hostile(2);
    steps.deploying(DeployOutcome::Complete);
    let jitter = FakeJitter(0);
    let pause = RecordingPause::default();
    let (logs, subscriber) = capture_logs();
    let guard = set_default(subscriber);
    run_passes(&steps, settings(42, 77), &jitter, &pause)
        .await
        .expect("an ahead-of-head warning never blocks deployment");
    drop(guard);
    assert_eq!(steps.ahead_checks(), 1);
    assert!(
        logs.contents().contains("startup continues"),
        "{}",
        logs.contents()
    );
}

#[tokio::test(start_paused = true)]
async fn no_warning_without_ahead_history() {
    let steps = ScriptedSteps::default();
    steps.deploying(DeployOutcome::Complete);
    let jitter = FakeJitter(0);
    let pause = RecordingPause::default();
    let (logs, subscriber) = capture_logs();
    let guard = set_default(subscriber);
    run_passes(&steps, settings(42, 77), &jitter, &pause)
        .await
        .expect("a clean history deploys without warnings");
    drop(guard);
    assert!(
        !logs.contents().contains("startup continues"),
        "{}",
        logs.contents()
    );
}

#[tokio::test]
async fn completion_evidence_before_permission_classification() {
    with_schema(|database| async move {
        let source = raised_database_error(&database, "permission denied for table x").await;
        let steps = ScriptedSteps::default();
        steps.deploying(DeployOutcome::FailedStep(FailedStep {
            name: "20240101000000_a".to_owned(),
            source: StepSource::Database(source),
        }));
        steps.recovering(Ok(true));
        steps.deploying(DeployOutcome::Complete);
        let jitter = FakeJitter(0);
        let pause = RecordingPause::default();
        run_passes(&steps, settings(42, 77), &jitter, &pause)
            .await
            .expect("verified completion outranks the permission rejection");
        assert_eq!(
            steps.events(),
            vec![
                "ahead",
                "deploy",
                "recover:20240101000000_a",
                "ahead",
                "deploy"
            ]
        );
    })
    .await;
}

#[test]
fn legacy_cli_flag_precedence() {
    assert!(!resolve_v2_migration_resolver(true, None));
    assert!(!resolve_v2_migration_resolver(true, Some("true")));
    assert!(resolve_v2_migration_resolver(false, None));
    assert!(resolve_v2_migration_resolver(false, Some("true")));
    assert!(!resolve_v2_migration_resolver(false, Some("false")));
    assert!(!resolve_v2_migration_resolver(false, Some("")));
    assert!(!resolve_v2_migration_resolver(false, Some("on")));
    assert!(resolve_v2_migration_resolver(false, Some("1")));
    assert!(resolve_v2_migration_resolver(false, Some("t")));
    assert!(resolve_v2_migration_resolver(false, Some("y")));
    assert!(resolve_v2_migration_resolver(false, Some("yes")));
}

#[tokio::test]
async fn native_legacy_mode_warns_and_runs_v2() {
    with_schema(|database| async move {
        let catalog = [migration(
            "20240101000000_create",
            "CREATE TABLE legacy_mode_probe (id integer)",
        )];
        let jitter = FakeJitter(0);
        let url = format!(
            "{}&schema={}",
            database.url.expose_secret(),
            database.schema
        );
        let setup = inputs(Some(&url), Some(&url), &catalog, &jitter);
        let (logs, subscriber) = capture_logs();
        let guard = set_default(subscriber);
        setup_database_v2(
            &setup,
            SetupRequest {
                use_v2_resolver: false,
                db_push_requested: false,
            },
        )
        .await
        .expect("legacy mode still runs the native v2 resolver");
        drop(guard);
        assert!(relation_exists(&database, "legacy_mode_probe").await);
        assert!(
            logs.contents().contains("native v2 resolver"),
            "{}",
            logs.contents()
        );
    })
    .await;
}

#[tokio::test]
async fn native_requested_db_push_rejected_before_mutation() {
    let jitter = FakeJitter(0);
    let setup = inputs(
        Some("postgresql://user:pass@127.0.0.1:1/db"),
        None,
        &[],
        &jitter,
    );
    let error = setup_database_v2(
        &setup,
        SetupRequest {
            use_v2_resolver: true,
            db_push_requested: true,
        },
    )
    .await
    .expect_err("db push is refused");
    assert!(matches!(error, RecoveryError::DbPushRequested));
    assert!(error.to_string().contains("pinned migrations"), "{error}");
    assert!(
        error
            .to_string()
            .contains("baseline an already-current schema"),
        "{error}"
    );
    let absent = inputs(None, None, &[], &jitter);
    let absent_error = setup_database_v2(
        &absent,
        SetupRequest {
            use_v2_resolver: true,
            db_push_requested: true,
        },
    )
    .await
    .expect_err("the push refusal precedes the DATABASE_URL check");
    assert!(matches!(absent_error, RecoveryError::DbPushRequested));
}

#[tokio::test]
async fn replica_identity_disabled_no_call() {
    let called = AtomicBool::new(false);
    let applied = replica_identity_if_requested(false, || {
        called.store(true, Ordering::SeqCst);
        ready(true)
    })
    .await;
    assert!(!applied);
    assert!(!called.load(Ordering::SeqCst));
}

#[tokio::test]
async fn replica_identity_result_forwarded() {
    assert!(replica_identity_if_requested(true, || ready(true)).await);
    assert!(!replica_identity_if_requested(true, || ready(false)).await);
}
