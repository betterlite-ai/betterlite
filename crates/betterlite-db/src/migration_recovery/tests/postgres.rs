use std::slice::from_ref;
use std::time::Duration;

use crate::migration_ledger::ledger_reads::FailedLookup;

use crate::migration_ledger::lock::PRISMA_LOCK_KEY;
use crate::migration_recovery::RecoveryError;
use crate::migration_recovery::outcome::{DeployFailure, DeployOutcome, FailedIdentity};
use crate::migration_recovery::policy::{RecoverySteps, StepFuture};
use crate::migration_recovery::steps::PgSteps;
use crate::migration_recovery::tests::database::{
    LedgerRow, create_ledger, execute, insert_history, insert_row, ledger_table, migration,
    relation_exists, stored_row, with_schema,
};
use crate::migration_recovery::tests::support::{FakeJitter, ScriptedSteps};

const DEPLOY_TIMEOUT: Duration = Duration::from_secs(30);

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

pub(super) struct HybridSteps<'input> {
    pub(super) scripted: ScriptedSteps,
    pub(super) pg: PgSteps<'input>,
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
async fn postgres_ahead_history_reports_names() {
    with_schema(|database| async move {
        create_ledger(&database).await;
        insert_row(
            &database,
            &LedgerRow {
                id: "00000000-0000-0000-0000-000000000008",
                name: "20990101000000_future",
                checksum: "eeee",
                steps: 1,
                logs: None,
                finished: true,
                rolled_back: false,
                started_at: "2024-01-02 03:04:05+00",
            },
        )
        .await;
        let jitter = FakeJitter(0);
        let steps = database.steps(&[], &jitter);
        let ahead = steps.warn_ahead().await;
        assert!(
            ahead.iter().any(|name| name == "20990101000000_future"),
            "{ahead:?}"
        );
    })
    .await;
}

#[tokio::test]
async fn postgres_database_failure_is_manual() {
    with_schema(|database| async move {
        execute(&database, "CREATE TABLE _prisma_migrations (bogus text)").await;
        let catalog = [migration("20240101000000_a", "SELECT 1;")];
        let jitter = FakeJitter(0);
        let steps = database.steps(&catalog, &jitter);
        let outcome = steps
            .deploy(DEPLOY_TIMEOUT)
            .await
            .expect("a typed deploy outcome");
        let DeployOutcome::Failed(failure) = outcome else {
            panic!("a raw database failure is a manual outcome: {outcome:?}");
        };
        let DeployFailure::Database(source) = failure else {
            panic!("the raw failure keeps its database error: {failure:?}");
        };
        assert!(
            source
                .as_db_error()
                .expect("a database error")
                .message()
                .contains("does not exist"),
            "{source}"
        );
    })
    .await;
}

#[tokio::test]
async fn postgres_failed_ledger_write_reports_both_errors() {
    with_schema(|database| async move {
        create_ledger(&database).await;
        let catalog = [migration(
            "20240101000000_drop",
            "BEGIN; DROP TABLE _prisma_migrations; COMMIT; SELECT 1 / 0;",
        )];
        let jitter = FakeJitter(0);
        let steps = database.steps(&catalog, &jitter);
        let outcome = steps
            .deploy(DEPLOY_TIMEOUT)
            .await
            .expect("a typed deploy outcome");
        let DeployOutcome::Failed(failure) = outcome else {
            panic!("a failed ledger write is a manual outcome: {outcome:?}");
        };
        let DeployFailure::MigrationLog {
            name,
            source,
            ledger,
        } = failure
        else {
            panic!("the failure keeps the migration and ledger errors: {failure:?}");
        };
        assert_eq!(name, "20240101000000_drop");
        assert!(
            source
                .as_db_error()
                .expect("a database error")
                .message()
                .contains("division by zero")
                && ledger
                    .as_db_error()
                    .expect("a database error")
                    .message()
                    .contains("does not exist"),
            "{source} {ledger}"
        );
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
        assert!(matches!(outcome, DeployOutcome::SchemaNotBaselined));
        assert!(!relation_exists(&database, "_prisma_migrations").await);
    })
    .await;
}

#[tokio::test]
async fn postgres_lock_contention_is_typed() {
    with_schema(|database| async move {
        let jitter = FakeJitter(0);
        let steps = database.steps(&[], &jitter);
        let held = steps
            .deploy_lock_held()
            .await
            .expect("the lock state reads");
        assert!(!held, "no session holds the Prisma migration lock yet");
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
        // A generous budget lets the statement start and be canceled by statement_timeout (57014).
        let outcome = steps
            .deploy(Duration::from_secs(2))
            .await
            .expect("a typed deploy outcome");
        assert!(matches!(outcome, DeployOutcome::TimedOut));
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
        assert!(matches!(outcome, DeployOutcome::Complete));
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

#[tokio::test]
async fn postgres_identity_survives_concurrent_rollback() {
    with_schema(|database| async move {
        create_ledger(&database).await;
        let failed = migration("20240101000000_a", "SELECT 1;");
        insert_history(
            &database,
            &failed,
            "00000000-0000-0000-0000-000000000009",
            "2024-01-02 03:04:05+00",
            false,
        )
        .await;
        let jitter = FakeJitter(0);
        let steps = database.steps(from_ref(&failed), &jitter);
        steps
            .roll_back(&failed.name)
            .await
            .expect("a concurrent replica rolls the row back");
        let identity = steps
            .unfinished_identity(&failed.name)
            .await
            .expect("the identity read")
            .expect("the newest row still carries its identity");
        assert_eq!(identity.name, failed.name);
        assert_eq!(identity.started_at, "2024-01-02 03:04:05+00");
        let lookup = steps.failed_history(&identity).await;
        let FailedLookup::Found(recovered) = lookup else {
            panic!("the recovered row is still found by its exact identity: {lookup:?}");
        };
        assert!(recovered.recovered);
    })
    .await;
}
