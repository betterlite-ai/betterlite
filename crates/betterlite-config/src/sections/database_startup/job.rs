use std::future::Future;
use std::pin::Pin;

use betterlite_db::migration_deploy::catalog::Migration;
use betterlite_db::migration_ledger::lock::JitterSource;
use betterlite_db::migration_recovery::RecoveryError;

use super::startup::{StartupSettings, build_request_log_indexes, setup_database};

pub type BuildFuture<'service> = Pin<Box<dyn Future<Output = bool> + Send + 'service>>;

// The request-log index build is its own port so the job's wait for it is observable from a test
pub trait IndexBuild: Send + Sync {
    fn build(&self) -> BuildFuture<'_>;
}

pub struct RequestLogIndexBuild<'settings> {
    pub settings: &'settings StartupSettings,
}

impl IndexBuild for RequestLogIndexBuild<'_> {
    fn build(&self) -> BuildFuture<'_> {
        Box::pin(build_request_log_indexes(self.settings))
    }
}

// migrations/run.py exits 0 only when setup and the request-log index build both succeed
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobOutcome {
    Completed,
    SetupFailed,
    IndexJobFailed,
}

// proxy_cli.py exits 2 for a RuntimeError and 1 for a setup that returned False after its retries
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupFailure {
    RetriesExhausted,
    Unrecoverable,
}

// Only an exhausted attempt budget is the retried-False exit; every definite failure is raised
#[must_use]
pub const fn classify(error: &RecoveryError) -> SetupFailure {
    match error {
        RecoveryError::AttemptsExhausted { .. } => SetupFailure::RetriesExhausted,
        RecoveryError::DatabaseUrlRequired
        | RecoveryError::DbPushRequested
        | RecoveryError::Url { .. }
        | RecoveryError::Translation { .. }
        | RecoveryError::ConnectionOptions { .. }
        | RecoveryError::Connection { .. }
        | RecoveryError::DeployConnection { .. }
        | RecoveryError::Pool { .. }
        | RecoveryError::Database { .. }
        | RecoveryError::DeployTimeout
        | RecoveryError::QueryTimeout { .. }
        | RecoveryError::Lock(_)
        | RecoveryError::Ledger(_)
        | RecoveryError::Inert(_)
        | RecoveryError::Baseline(_)
        | RecoveryError::Budget(_)
        | RecoveryError::UnverifiedCompletion { .. }
        | RecoveryError::InsufficientPrivileges { .. }
        | RecoveryError::ManualIntervention { .. } => SetupFailure::Unrecoverable,
    }
}

// Mirrors ProxyExtrasDBManager.run_migration_job: setup first, then the index build awaited in place
pub async fn run_migration_job<Build>(
    settings: &StartupSettings,
    catalog: &[Migration],
    schema_prisma: &[u8],
    jitter: &dyn JitterSource,
    build: &Build,
) -> Result<JobOutcome, RecoveryError>
where
    Build: IndexBuild,
{
    if let Err(error) = setup_database(settings, catalog, schema_prisma, jitter).await {
        return match classify(&error) {
            SetupFailure::RetriesExhausted => Ok(JobOutcome::SetupFailed),
            SetupFailure::Unrecoverable => Err(error),
        };
    }
    if build.build().await {
        Ok(JobOutcome::Completed)
    } else {
        Ok(JobOutcome::IndexJobFailed)
    }
}
