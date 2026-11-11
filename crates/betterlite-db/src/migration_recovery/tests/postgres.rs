use std::slice::from_ref;
use std::time::Duration;

use tracing::subscriber::set_default;

use crate::migration_ledger::ledger_reads::FailedLookup;

use crate::migration_ledger::lock::PRISMA_LOCK_KEY;
use crate::migration_recovery::RecoveryError;
use crate::migration_recovery::outcome::{DeployOutcome, FailedIdentity, FailedStep};
use crate::migration_recovery::policy::{PassSettings, RecoverySteps, StepFuture, run_passes};
use crate::migration_recovery::steps::PgSteps;
use crate::migration_recovery::tests::database::{
    LedgerRow, create_ledger, execute, insert_history, insert_row, ledger_table, migration,
    relation_exists, stored_row, with_schema,
};
use crate::migration_recovery::tests::support::{
    FakeJitter, RecordingPause, ScriptedSteps, capture_logs,
};

const DEPLOY_TIMEOUT: Duration = Duration::from_secs(30);

#[tokio::test]
async fn postgres_rollback_marker_preserves_history() {
    with_schema(|database| async move {
        create_ledger(&database).await;
        let failed = migration("20240101000000_a", "SELECT 1;");
        let completed = migration("20240101000000_b", "SELECT 2;");
        insert_history(
            &database,
            &failed,
            "00000000-0000-0000-0000-000000000001",
            "2024-01-02 03:04:05+00",
            false,
        )
        .await;
        insert_history(
            &database,
            &completed,
            "00000000-0000-0000-0000-000000000002",
            "2024-01-03 03:04:05+00",
            true,
        )
        .await;
        let before = stored_row(&database, &failed.name).await;
        let jitter = FakeJitter(0);
        let steps = database.steps(from_ref(&failed), &jitter);
        steps
            .roll_back(&failed.name)
            .await
            .expect("the best-effort marker write succeeds");
        let after = stored_row(&database, &failed.name).await;
        assert!(after.rolled_back);
        assert_eq!(after.finished, before.finished);
        assert_eq!(after.checksum, before.checksum);
        assert_eq!(after.logs, before.logs);
        assert_eq!(after.steps, before.steps);
        assert_eq!(after.started_at, before.started_at);
        let untouched = stored_row(&database, &completed.name).await;
        assert!(untouched.finished);
        assert!(!untouched.rolled_back);
    })
    .await;
}

#[tokio::test]
async fn completed_history_not_rewritten() {
    with_schema(|database| async move {
        create_ledger(&database).await;
        let completed = migration("20240101000000_done", "SELECT 1;");
        insert_row(
            &database,
            &LedgerRow {
                id: "00000000-0000-0000-0000-000000000003",
                name: &completed.name,
                checksum: &completed.checksum,
                steps: 1,
                logs: Some("ok"),
                finished: true,
                rolled_back: false,
                started_at: "2024-01-04 03:04:05+00",
            },
        )
        .await;
        let before = stored_row(&database, &completed.name).await;
        let jitter = FakeJitter(0);
        let steps = database.steps(from_ref(&completed), &jitter);
        steps
            .roll_back(&completed.name)
            .await
            .expect("the best-effort marker write succeeds");
        let after = stored_row(&database, &completed.name).await;
        assert!(after.finished);
        assert!(!after.rolled_back);
        assert_eq!(after.checksum, before.checksum);
        assert_eq!(after.logs, before.logs);
        assert_eq!(after.steps, before.steps);
        assert_eq!(after.started_at, before.started_at);
    })
    .await;
}

struct HybridSteps<'input> {
    scripted: ScriptedSteps,
    pg: PgSteps<'input>,
}

impl RecoverySteps for HybridSteps<'_> {
    fn deploy(&self, timeout: Duration) -> StepFuture<'_, Result<DeployOutcome, RecoveryError>> {
        self.scripted.deploy(timeout)
    }

    fn recover_completed<'input>(
        &'input self,
        name: &'input str,
    ) -> StepFuture<'input, Result<bool, RecoveryError>> {
        self.scripted.recover_completed(name)
    }

    fn baseline(&self) -> StepFuture<'_, Result<(), RecoveryError>> {
        self.scripted.baseline()
    }

    fn failed_history<'input>(
        &'input self,
        identity: &'input FailedIdentity,
    ) -> StepFuture<'input, FailedLookup> {
        self.scripted.failed_history(identity)
    }

    fn roll_back<'input>(
        &'input self,
        name: &'input str,
    ) -> StepFuture<'input, Result<(), RecoveryError>> {
        self.pg.roll_back(name)
    }

    fn warn_ahead(&self) -> StepFuture<'_, Vec<String>> {
        self.scripted.warn_ahead()
    }
}

#[tokio::test]
async fn postgres_failed_marker_write_is_silent() {
    with_schema(|database| async move {
        let catalog = [migration("20240101000000_a", "SELECT 1;")];
        let jitter = FakeJitter(0);
        let hybrid = HybridSteps {
            scripted: ScriptedSteps::default(),
            pg: database.steps(&catalog, &jitter),
        };
        hybrid
            .scripted
            .deploying(DeployOutcome::FailedStep(FailedStep {
                name: Some("20240101000000_a".to_owned()),
                sqlstate: Some("40P01".to_owned()),
                detail: "deadlock detected".to_owned(),
            }));
        hybrid.scripted.deploying(DeployOutcome::Complete);
        let pause = RecordingPause::default();
        let settings = PassSettings {
            deploy_timeout: DEPLOY_TIMEOUT,
            lock_timeout: Duration::from_secs(30),
        };
        let (logs, subscriber) = capture_logs();
        let guard = set_default(subscriber);
        run_passes(&hybrid, settings, &jitter, &pause)
            .await
            .expect("a failed marker write leaves the attempt consumed and the pass retries");
        drop(guard);
        assert_eq!(hybrid.scripted.deploy_timeouts().len(), 2);
        assert_eq!(pause.delays().len(), 1);
        assert!(!logs.contents().contains("WARN"), "{}", logs.contents());
    })
    .await;
}

#[tokio::test]
async fn postgres_partial_ddl_not_undone() {
    with_schema(|database| async move {
        let catalog = [
            migration(
                "20240101000000_create",
                "CREATE TABLE survivor (id integer)",
            ),
            migration("20240101000001_break", "SELECT 1 / 0"),
        ];
        let jitter = FakeJitter(0);
        let steps = database.steps(&catalog, &jitter);
        let outcome = steps
            .deploy(DEPLOY_TIMEOUT)
            .await
            .expect("a typed deploy outcome");
        assert!(
            matches!(outcome, DeployOutcome::FailedStep(_)),
            "{outcome:?}"
        );
        assert!(relation_exists(&database, "survivor").await);
        steps
            .roll_back(&catalog[1].name)
            .await
            .expect("the marker write succeeds");
        assert!(
            relation_exists(&database, "survivor").await,
            "resolving history never undoes committed DDL"
        );
        let broken = stored_row(&database, &catalog[1].name).await;
        assert!(broken.rolled_back);
        assert!(!broken.finished);
        let kept = stored_row(&database, &catalog[0].name).await;
        assert!(kept.finished);
        assert!(!kept.rolled_back);
    })
    .await;
}

#[tokio::test]
async fn postgres_nonempty_schema_needs_baseline() {
    with_schema(|database| async move {
        execute(&database, "CREATE TABLE preexisting (id integer)").await;
        let jitter = FakeJitter(0);
        let steps = database.steps(&[], &jitter);
        let outcome = steps
            .deploy(DEPLOY_TIMEOUT)
            .await
            .expect("a typed deploy outcome");
        assert_eq!(outcome, DeployOutcome::SchemaNotBaselined);
        assert!(!relation_exists(&database, "_prisma_migrations").await);
    })
    .await;
}

#[tokio::test]
async fn postgres_lock_contention_is_typed() {
    with_schema(|database| async move {
        let jitter = FakeJitter(0);
        let steps = database.steps(&[], &jitter);
        let holder = database.pool.get().await.expect("a holder session");
        holder
            .execute("SELECT pg_advisory_lock($1)", &[&PRISMA_LOCK_KEY])
            .await
            .expect("the test holds Prisma's migration lock");
        let outcome = steps
            .deploy(Duration::from_millis(200))
            .await
            .expect("a typed deploy outcome");
        assert!(
            matches!(outcome, DeployOutcome::Contention { .. }),
            "{outcome:?}"
        );
        holder
            .execute("SELECT pg_advisory_unlock($1)", &[&PRISMA_LOCK_KEY])
            .await
            .expect("the holder releases the lock");
    })
    .await;
}

#[tokio::test]
async fn postgres_slow_statement_is_a_deploy_timeout() {
    with_schema(|database| async move {
        let catalog = [migration("20240101000000_slow", "SELECT pg_sleep(30)")];
        let jitter = FakeJitter(0);
        let steps = database.steps(&catalog, &jitter);
        let outcome = steps
            .deploy(Duration::from_millis(250))
            .await
            .expect("a typed deploy outcome");
        assert_eq!(outcome, DeployOutcome::TimedOut);
    })
    .await;
}

#[tokio::test]
async fn postgres_unbaselined_with_empty_schema_deploys() {
    with_schema(|database| async move {
        let catalog = [migration(
            "20240101000000_first",
            "CREATE TABLE first_migration (id integer)",
        )];
        let jitter = FakeJitter(0);
        let steps = database.steps(&catalog, &jitter);
        let outcome = steps
            .deploy(DEPLOY_TIMEOUT)
            .await
            .expect("a typed deploy outcome");
        assert_eq!(outcome, DeployOutcome::Complete);
        assert!(relation_exists(&database, "first_migration").await);
        let row = stored_row(&database, &catalog[0].name).await;
        assert!(row.finished);
    })
    .await;
}

#[tokio::test]
async fn postgres_recovery_writes_into_the_writer_namespace() {
    with_schema(|database| async move {
        create_ledger(&database).await;
        let failed = migration("20240101000000_a", "SELECT 1;");
        insert_row(
            &database,
            &LedgerRow {
                id: "00000000-0000-0000-0000-000000000004",
                name: &failed.name,
                checksum: &failed.checksum,
                steps: 1,
                logs: Some(""),
                finished: false,
                rolled_back: false,
                started_at: "2024-01-02 03:04:05+00",
            },
        )
        .await;
        let jitter = FakeJitter(0);
        let steps = database.steps(from_ref(&failed), &jitter);
        assert!(
            steps
                .recover_completed(&failed.name)
                .await
                .expect("recovery")
        );
        let identity = FailedIdentity {
            name: failed.name.clone(),
            started_at: stored_row(&database, &failed.name).await.started_at,
        };
        let lookup = steps.failed_history(&identity).await;
        assert!(
            matches!(lookup, FailedLookup::Found(_)),
            "the ledger read uses the DATABASE_URL namespace, not the search path"
        );
    })
    .await;
}

#[tokio::test]
async fn postgres_rollback_targets_only_the_named_migration() {
    with_schema(|database| async move {
        create_ledger(&database).await;
        let failed = migration("20240101000000_a", "SELECT 1;");
        insert_history(
            &database,
            &failed,
            "00000000-0000-0000-0000-000000000005",
            "2024-01-02 03:04:05+00",
            false,
        )
        .await;
        let other = migration("20240101000000_other", "SELECT 1;");
        insert_history(
            &database,
            &other,
            "00000000-0000-0000-0000-000000000006",
            "2024-01-02 04:04:05+00",
            false,
        )
        .await;
        let jitter = FakeJitter(0);
        let steps = database.steps(from_ref(&failed), &jitter);
        steps
            .roll_back(&failed.name)
            .await
            .expect("the marker write");
        assert!(stored_row(&database, &failed.name).await.rolled_back);
        assert!(!stored_row(&database, &other.name).await.rolled_back);
    })
    .await;
}

#[tokio::test]
async fn postgres_unfinished_identity_reads_started_at() {
    with_schema(|database| async move {
        create_ledger(&database).await;
        execute(
            &database,
            &format!(
                "INSERT INTO {} (id, checksum, migration_name, logs, started_at, applied_steps_count) \
                 VALUES ('00000000-0000-0000-0000-000000000007', 'dddd', \
                 '20240101000000_pending', 'ERROR: deadlock detected', \
                 '2024-05-06 07:08:09+00', 0)",
                ledger_table(&database.schema)
            ),
        )
        .await;
        let catalog = [migration("20240101000000_pending", "SELECT 1;")];
        let jitter = FakeJitter(0);
        let steps = database.steps(&catalog, &jitter);
        let outcome = steps
            .deploy(DEPLOY_TIMEOUT)
            .await
            .expect("a typed deploy outcome");
        let DeployOutcome::UnfinishedHistory { identity } = outcome else {
            panic!("an unfinished row stops the deploy as typed history: {outcome:?}");
        };
        let found = identity.expect("the exact ledger identity");
        assert_eq!(found.name, "20240101000000_pending");
        assert_eq!(found.started_at, "2024-05-06 07:08:09+00");
    })
    .await;
}
