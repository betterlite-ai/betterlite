use tokio_postgres::Error as PostgresError;

use super::super::outcome::{
    CANCELED_SQLSTATE, DeployFailure, DeployOutcome, FailedStep, StepSource, database_detail,
    is_deadlock, sqlstate_of,
};

pub(super) fn step_outcome(name: String, source: PostgresError) -> DeployOutcome {
    if sqlstate_of(&source).as_deref() == Some(CANCELED_SQLSTATE) {
        DeployOutcome::TimedOut
    } else {
        DeployOutcome::FailedStep(FailedStep {
            name,
            source: StepSource::Database(source),
        })
    }
}

pub(super) fn database_outcome(source: PostgresError) -> DeployOutcome {
    if sqlstate_of(&source).as_deref() == Some(CANCELED_SQLSTATE) {
        return DeployOutcome::TimedOut;
    }
    if is_deadlock(sqlstate_of(&source).as_deref(), &database_detail(&source)) {
        return DeployOutcome::Deadlock;
    }
    DeployOutcome::Failed(DeployFailure::Database(source))
}

pub(super) fn failed_after_step(
    name: String,
    source: PostgresError,
    ledger: PostgresError,
) -> DeployOutcome {
    if sqlstate_of(&source).as_deref() == Some(CANCELED_SQLSTATE) {
        return DeployOutcome::TimedOut;
    }
    DeployOutcome::Failed(DeployFailure::MigrationLog {
        name,
        source,
        ledger,
    })
}
