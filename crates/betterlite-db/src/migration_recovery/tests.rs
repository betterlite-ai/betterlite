#![cfg(test)]

mod budget;
mod database;
mod failure;
mod postgres;
mod setup;
mod support;

use std::slice::from_ref;
use std::time::Duration;

use tracing::subscriber::set_default;

use crate::migration_recovery::outcome::{DeployOutcome, FailedStep, StepSource};
use crate::migration_recovery::policy::{PassSettings, RecoverySteps as _, run_passes};
use crate::migration_recovery::tests::database::{
    create_ledger, insert_history, migration, raised_database_error, relation_exists, stored_row,
    with_schema,
};
use crate::migration_recovery::tests::postgres::HybridSteps;
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
async fn postgres_failed_marker_write_is_silent() {
    with_schema(|database| async move {
        let deadlock = raised_database_error(&database, "deadlock detected").await;
        let catalog = [migration("20240101000000_a", "SELECT 1;")];
        let jitter = FakeJitter(0);
        let hybrid = HybridSteps {
            scripted: ScriptedSteps::default(),
            pg: database.steps(&catalog, &jitter),
        };
        hybrid
            .scripted
            .deploying(DeployOutcome::FailedStep(FailedStep {
                name: "20240101000000_a".to_owned(),
                source: StepSource::Database(deadlock),
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
        let DeployOutcome::FailedStep(step) = outcome else {
            panic!("a broken migration is a typed step failure: {outcome:?}");
        };
        assert_eq!(step.name, "20240101000001_break");
        assert_eq!(step.sqlstate().as_deref(), Some("22012"), "{step:?}");
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
