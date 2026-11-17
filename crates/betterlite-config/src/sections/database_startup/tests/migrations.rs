use betterlite_db::migration_recovery::RecoveryError;

use super::postgres::{NoJitter, catalog_request_log_indexes, with_schema};
use crate::sections::database_startup::job::{
    BuildFuture, IndexBuild, JobOutcome, RequestLogIndexBuild, SetupFailure, classify,
    run_migration_job,
};
use crate::sections::database_startup::packaged;
use crate::sections::database_startup::startup::{
    StartupSwitch, build_request_log_indexes, setup_database,
};

// An index-build port with a fixed answer, so the job's outcome mapping is observable on its own.
struct FixedBuild(bool);

impl IndexBuild for FixedBuild {
    fn build(&self) -> BuildFuture<'_> {
        let built = self.0;
        Box::pin(async move { built })
    }
}

#[test]
fn only_an_exhausted_attempt_budget_is_the_retried_false_exit() {
    let exhausted = RecoveryError::AttemptsExhausted {
        message: "no attempts left".to_owned(),
    };
    assert_eq!(classify(&exhausted), SetupFailure::RetriesExhausted);
    assert_eq!(
        classify(&RecoveryError::DbPushRequested),
        SetupFailure::Unrecoverable
    );
}

#[tokio::test]
async fn packaged_setup_builds_the_indexes_and_applies_the_identity() {
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
        assert_eq!(database.spend_log_replica_identity().await, "d");
        assert!(build_request_log_indexes(&settings).await);
        assert_eq!(
            database.present_request_log_indexes().await,
            catalog_request_log_indexes(),
            "the build created every request-log index in the URL's schema"
        );

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
        assert_eq!(database.spend_log_replica_identity().await, "f");
    })
    .await;
}

#[tokio::test]
async fn the_migration_job_builds_the_indexes_and_reports_a_failed_build() {
    with_schema(|database| async move {
        let settings = database.settings();
        let real_build = RequestLogIndexBuild {
            settings: &settings,
        };
        let completed = run_migration_job(
            &settings,
            &packaged::migrations(),
            packaged::SCHEMA_PRISMA,
            &NoJitter,
            &real_build,
        )
        .await
        .expect("the job completes");
        assert_eq!(completed, JobOutcome::Completed);
        assert_eq!(
            database.present_request_log_indexes().await,
            catalog_request_log_indexes(),
            "the job awaited the index build"
        );

        let index_failure = run_migration_job(
            &settings,
            &packaged::migrations(),
            packaged::SCHEMA_PRISMA,
            &NoJitter,
            &FixedBuild(false),
        )
        .await
        .expect("an index failure is not a migration failure");
        assert_eq!(index_failure, JobOutcome::IndexJobFailed);
    })
    .await;
}
