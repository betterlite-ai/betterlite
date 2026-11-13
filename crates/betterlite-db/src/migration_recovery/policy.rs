use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use tokio::time::{Instant, sleep};

use crate::migration_ledger::budget::{MAX_MIGRATE_DEPLOY_ATTEMPTS, MigrateAttemptBudget};
use crate::migration_ledger::ledger_reads::{FailedLookup, FailedMigration};
use crate::migration_ledger::lock::JitterSource;

use super::RecoveryError;
use super::messages;
use super::outcome::{
    DEADLOCK_MARKER, DeployOutcome, FailedIdentity, FailedStep, is_deadlock, is_permission_error,
};

pub(super) type StepFuture<'step, T> = Pin<Box<dyn Future<Output = T> + Send + 'step>>;

pub(super) trait RecoverySteps: Send + Sync {
    fn deploy(&self, timeout: Duration) -> StepFuture<'_, Result<DeployOutcome, RecoveryError>>;
    fn recover_completed<'step>(
        &'step self,
        name: &'step str,
    ) -> StepFuture<'step, Result<bool, RecoveryError>>;
    fn baseline(&self) -> StepFuture<'_, Result<(), RecoveryError>>;
    fn failed_history<'step>(
        &'step self,
        identity: &'step FailedIdentity,
    ) -> StepFuture<'step, FailedLookup>;
    fn roll_back<'step>(
        &'step self,
        name: &'step str,
    ) -> StepFuture<'step, Result<(), RecoveryError>>;
    fn warn_ahead(&self) -> StepFuture<'_, Vec<String>>;
}

pub(super) trait RetryPause: Send + Sync {
    fn pause(&self, delay: Duration) -> StepFuture<'_, ()>;
}

pub(super) struct TokioPause;

impl RetryPause for TokioPause {
    fn pause(&self, delay: Duration) -> StepFuture<'_, ()> {
        Box::pin(sleep(delay))
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct PassSettings {
    pub deploy_timeout: Duration,
    pub lock_timeout: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PassOutcome {
    Complete,
    Checkpoint,
}

enum PassStep {
    Finished(PassOutcome),
    Retry(MigrateAttemptBudget),
}

// Every checkpoint restarts both budgets, so one successful recovery never counts against the next pass
pub(super) async fn run_passes(
    steps: &dyn RecoverySteps,
    settings: PassSettings,
    jitter: &dyn JitterSource,
    pause: &dyn RetryPause,
) -> Result<(), RecoveryError> {
    loop {
        let budget = MigrateAttemptBudget {
            attempts_left: MAX_MIGRATE_DEPLOY_ATTEMPTS,
            contention_left: settings.lock_timeout,
        };
        let outcome =
            run_database_v2(steps, budget, settings.deploy_timeout, jitter, pause).await?;
        if outcome == PassOutcome::Complete {
            return Ok(());
        }
    }
}

pub(super) async fn run_database_v2(
    steps: &dyn RecoverySteps,
    mut budget: MigrateAttemptBudget,
    deploy_timeout: Duration,
    jitter: &dyn JitterSource,
    pause: &dyn RetryPause,
) -> Result<PassOutcome, RecoveryError> {
    let hostile = steps.warn_ahead().await;
    if !hostile.is_empty() {
        tracing::warn!(
            count = hostile.len(),
            "the database has migrations this build does not ship; startup continues"
        );
    }
    while budget.attempts_left > 0_i32 {
        let attempt_started = Instant::now();
        let outcome = steps.deploy(deploy_timeout).await?;
        let step = next_step(
            steps,
            outcome,
            budget,
            attempt_started.elapsed(),
            deploy_timeout,
        )
        .await?;
        match step {
            PassStep::Finished(pass) => return Ok(pass),
            PassStep::Retry(next) => {
                if next.attempts_left < budget.attempts_left {
                    let drawn = Duration::from_nanos(jitter.uniform_nanos(10_000_000_000));
                    pause
                        .pause(
                            Duration::from_secs(5)
                                .saturating_add(Duration::from_secs(drawn.as_secs())),
                        )
                        .await;
                }
                budget = next;
            }
        }
    }
    Err(RecoveryError::AttemptsExhausted {
        message: messages::attempts_exhausted(),
    })
}

async fn next_step(
    steps: &dyn RecoverySteps,
    outcome: DeployOutcome,
    budget: MigrateAttemptBudget,
    elapsed: Duration,
    deploy_timeout: Duration,
) -> Result<PassStep, RecoveryError> {
    if let Some(name) = checkpoint_name(&outcome)
        && steps.recover_completed(name).await?
    {
        return Ok(PassStep::Finished(PassOutcome::Checkpoint));
    }
    match outcome {
        DeployOutcome::Complete => {
            tracing::info!("all pending database migrations are complete");
            Ok(PassStep::Finished(PassOutcome::Complete))
        }
        DeployOutcome::SchemaNotBaselined => {
            steps.baseline().await?;
            Ok(PassStep::Finished(PassOutcome::Checkpoint))
        }
        DeployOutcome::TimedOut => {
            timeout_warning(budget.attempt_number(), deploy_timeout);
            Ok(PassStep::Retry(budget.spend()))
        }
        DeployOutcome::UnfinishedHistory { identity } => Ok(PassStep::Retry(
            unfinished_history_budget(steps, identity, budget).await?,
        )),
        DeployOutcome::FailedStep(step) => Ok(PassStep::Retry(
            failed_step_budget(steps, step, budget).await?,
        )),
        DeployOutcome::Deadlock => Ok(PassStep::Retry(deadlock_attempt(budget))),
        DeployOutcome::Contention { detail } => Ok(PassStep::Retry(contention_attempt(
            budget, elapsed, &detail,
        )?)),
        DeployOutcome::Failed(failure) => {
            let message = messages::manual_for_failure(&failure);
            Err(RecoveryError::ManualIntervention {
                message,
                source: failure.into_cause(),
            })
        }
    }
}

fn checkpoint_name(outcome: &DeployOutcome) -> Option<&str> {
    match outcome {
        DeployOutcome::UnfinishedHistory { identity } => {
            identity.as_ref().map(|found| found.name.as_str())
        }
        DeployOutcome::FailedStep(step) => Some(step.name.as_str()),
        DeployOutcome::Complete
        | DeployOutcome::TimedOut
        | DeployOutcome::SchemaNotBaselined
        | DeployOutcome::Deadlock
        | DeployOutcome::Contention { .. }
        | DeployOutcome::Failed(_) => None,
    }
}

fn timeout_warning(attempt: i64, deploy_timeout: Duration) {
    tracing::warn!(
        "migration deploy attempt {attempt} timed out after {}s, retrying. Raise LITELLM_PRISMA_MIGRATE_DEPLOY_TIMEOUT if this database needs longer to apply its pending migrations.",
        deploy_timeout.as_secs()
    );
}

fn deadlock_attempt(budget: MigrateAttemptBudget) -> MigrateAttemptBudget {
    let attempt = budget.attempt_number();
    tracing::info!(
        "migration deploy attempt {attempt} deadlocked against a concurrent deploy, retrying"
    );
    budget.spend()
}

fn contention_attempt(
    budget: MigrateAttemptBudget,
    elapsed: Duration,
    detail: &str,
) -> Result<MigrateAttemptBudget, RecoveryError> {
    if is_deadlock(None, detail) {
        return Ok(deadlock_attempt(budget));
    }
    tracing::info!(
        "waiting for the migration advisory lock held by another process; contention does not spend a failure attempt"
    );
    Ok(budget.after_contention(elapsed)?)
}

async fn unfinished_history_budget(
    steps: &dyn RecoverySteps,
    reported: Option<FailedIdentity>,
    budget: MigrateAttemptBudget,
) -> Result<MigrateAttemptBudget, RecoveryError> {
    let Some(identity) = reported else {
        return Err(RecoveryError::UnverifiedCompletion {
            message: messages::unverified_completion(
                None,
                "the reported migration has no readable history identity",
            ),
        });
    };
    let lookup = steps.failed_history(&identity).await;
    if let FailedLookup::Found(FailedMigration { logs, recovered }) = &lookup {
        if logs.contains(DEADLOCK_MARKER) {
            tracing::info!(
                "migration {} failed in a concurrent migrate deploy deadlock race, rolling its ledger row back and retrying",
                identity.name
            );
            steps.roll_back(&identity.name).await?;
            return Ok(budget.spend());
        }
        if *recovered {
            tracing::info!(
                "migration {} started at {} was already rolled back or completed by a concurrent migrate deploy, retrying",
                identity.name,
                identity.started_at
            );
            return Ok(budget.spend());
        }
    }
    let reason = match lookup {
        FailedLookup::Unavailable => "the migration history could not be read",
        FailedLookup::Absent => "no unfinished migration history row was found",
        FailedLookup::Found(_) => "the failed migration row has no concurrent-recovery evidence",
    };
    Err(RecoveryError::UnverifiedCompletion {
        message: messages::unverified_completion(Some(&identity), reason),
    })
}

async fn failed_step_budget(
    steps: &dyn RecoverySteps,
    step: FailedStep,
    budget: MigrateAttemptBudget,
) -> Result<MigrateAttemptBudget, RecoveryError> {
    let sqlstate = step.sqlstate();
    let detail = step.detail();
    if is_permission_error(sqlstate.as_deref(), &detail) {
        return Err(RecoveryError::InsufficientPrivileges {
            message: messages::insufficient_privileges(&step),
            source: step.source.into_cause(),
        });
    }
    if is_deadlock(sqlstate.as_deref(), &detail) {
        tracing::info!(
            "migration {} deadlocked against a concurrent migrate deploy, rolling its ledger row back and retrying",
            step.name
        );
        steps.roll_back(&step.name).await?;
        return Ok(budget.spend());
    }
    Err(RecoveryError::ManualIntervention {
        message: messages::manual_for_step(&step),
        source: step.source.into_cause(),
    })
}
