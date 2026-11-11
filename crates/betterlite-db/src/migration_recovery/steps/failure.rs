use tokio_postgres::Error as PostgresError;

use super::super::outcome::{CANCELED_SQLSTATE, DeployOutcome, FailedStep, is_deadlock};

pub(super) fn step_outcome(name: String, source: &PostgresError) -> DeployOutcome {
    let sqlstate = sqlstate_of(source);
    if sqlstate.as_deref() == Some(CANCELED_SQLSTATE) {
        return DeployOutcome::TimedOut;
    }
    DeployOutcome::FailedStep(FailedStep {
        name: Some(name),
        sqlstate,
        detail: source.to_string(),
    })
}

pub(super) fn database_outcome(source: &PostgresError) -> DeployOutcome {
    let detail = source.to_string();
    let sqlstate = sqlstate_of(source);
    if sqlstate.as_deref() == Some(CANCELED_SQLSTATE) {
        return DeployOutcome::TimedOut;
    }
    if is_deadlock(sqlstate.as_deref(), &detail) {
        return DeployOutcome::Deadlock { detail };
    }
    DeployOutcome::Failed { detail }
}

pub(super) fn failed_after_step(
    name: &str,
    source: &PostgresError,
    ledger: &PostgresError,
) -> DeployOutcome {
    if sqlstate_of(source).as_deref() == Some(CANCELED_SQLSTATE) {
        return DeployOutcome::TimedOut;
    }
    DeployOutcome::Failed {
        detail: format!(
            "migration {name} failed ({source}) and recording the failure in the migration ledger also failed: {ledger}"
        ),
    }
}

fn sqlstate_of(source: &PostgresError) -> Option<String> {
    source
        .as_db_error()
        .map(|error| error.code().code().to_owned())
}
