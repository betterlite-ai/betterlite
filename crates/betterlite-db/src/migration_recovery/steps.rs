use std::time::Duration;

use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio::time::timeout;
use uuid::Uuid;

use crate::database_tls::options::native_connection_options;
use crate::migration_deploy::MigrationDeploy;
use crate::migration_deploy::catalog::Migration;
use crate::migration_drift::MigrationDriftCheck;
use crate::migration_drift::ahead::warn_if_db_ahead_of_head;
use crate::migration_ledger::baseline::{BaselineInputs, baseline_current_schema};
use crate::migration_ledger::connection::{CONNECT_TIMEOUT, ConnectionError, connect};
use crate::migration_ledger::inert::rollback_inert_migration;
use crate::migration_ledger::ledger_reads::{FailedLookup, look_up_failed_migration};
use crate::migration_ledger::lock::{JitterSource, MigrationCoordinator};
use crate::migration_ledger::recovery::recover_completed_migration;
use crate::schema_contract::SchemaContract;

use super::RecoveryError;
use super::outcome::{DeployOutcome, FailedIdentity};
use super::policy::{RecoverySteps, StepFuture};

mod deploy;
mod failure;

pub(super) struct PgStepsInputs<'input> {
    pub schema: String,
    pub catalog: &'input [Migration],
    pub prisma_url: SecretString,
    pub deploy_url: SecretString,
    pub lock_url: SecretString,
    pub database_url: SecretString,
    pub direct_url: Option<SecretString>,
    pub schema_prisma: &'input [u8],
    pub current_build: Option<&'input SchemaContract>,
    pub command_timeout: Duration,
    pub lock_timeout: Duration,
    pub jitter: &'input dyn JitterSource,
    pub next_id: fn() -> Uuid,
}

pub(super) struct PgSteps<'input> {
    deploy: MigrationDeploy,
    prisma_url: SecretString,
    deploy_url: SecretString,
    lock_url: SecretString,
    database_url: SecretString,
    direct_url: Option<SecretString>,
    schema_prisma: &'input [u8],
    current_build: Option<&'input SchemaContract>,
    command_timeout: Duration,
    lock_timeout: Duration,
    jitter: &'input dyn JitterSource,
    known: Vec<String>,
    pool: Pool,
}

impl<'input> PgSteps<'input> {
    pub(super) fn new(inputs: PgStepsInputs<'input>) -> Result<Self, RecoveryError> {
        let deploy = MigrationDeploy {
            schema: inputs.schema,
            catalog: inputs.catalog.to_vec(),
            next_id: inputs.next_id,
        };
        let known = deploy
            .catalog
            .iter()
            .map(|migration| migration.name.clone())
            .collect();
        let options = native_connection_options(inputs.deploy_url.expose_secret())
            .map_err(|source| RecoveryError::ConnectionOptions { source })?;
        let mut config = options.config;
        config.connect_timeout(CONNECT_TIMEOUT);
        let pool = Pool::builder(Manager::new(config, options.tls))
            .max_size(1)
            .build()
            .map_err(|source| RecoveryError::Pool { source })?;
        Ok(Self {
            deploy,
            prisma_url: inputs.prisma_url,
            deploy_url: inputs.deploy_url,
            lock_url: inputs.lock_url,
            database_url: inputs.database_url,
            direct_url: inputs.direct_url,
            schema_prisma: inputs.schema_prisma,
            current_build: inputs.current_build,
            command_timeout: inputs.command_timeout,
            lock_timeout: inputs.lock_timeout,
            jitter: inputs.jitter,
            known,
            pool,
        })
    }

    async fn recover_once(&self, name: &str) -> Result<bool, RecoveryError> {
        if !recoverable_name(name) {
            return Ok(false);
        }
        let Some(migration) = self
            .deploy
            .catalog
            .iter()
            .find(|migration| migration.name == name)
        else {
            return Ok(false);
        };
        let coordinator = MigrationCoordinator::acquire(
            connect(&self.lock_url).await?,
            self.lock_timeout,
            self.jitter,
        )
        .await?;
        let completed =
            recover_completed_migration(&coordinator, &self.deploy.schema, migration, self.jitter)
                .await?;
        if completed {
            coordinator.finish().await?;
            return Ok(true);
        }
        let rolled_back =
            rollback_inert_migration(&coordinator, &self.deploy.schema, migration, self.jitter)
                .await?;
        coordinator.finish().await?;
        if rolled_back {
            tracing::info!(
                migration = name,
                "rolled back the failed inert migration so the next deploy pass applies it"
            );
        }
        Ok(rolled_back)
    }

    async fn baseline_once(&self) -> Result<(), RecoveryError> {
        let coordinator = MigrationCoordinator::acquire(
            connect(&self.lock_url).await?,
            self.lock_timeout,
            self.jitter,
        )
        .await?;
        let inputs = BaselineInputs {
            schema: &self.deploy.schema,
            catalog: &self.deploy.catalog,
            schema_prisma: self.schema_prisma,
            current_build: self.current_build,
            command_timeout: self.command_timeout,
            next_id: self.deploy.next_id,
        };
        baseline_current_schema(&coordinator, &inputs, self.jitter).await?;
        coordinator.finish().await?;
        Ok(())
    }

    async fn roll_back_marker(&self, name: &str) -> Result<(), RecoveryError> {
        let client = connect(&self.deploy_url).await?;
        let table = format!(
            "\"{}\".\"_prisma_migrations\"",
            self.deploy.schema.replace('"', "\"\"")
        );
        let statement = format!(
            "UPDATE {table} SET rolled_back_at = current_timestamp \
             WHERE migration_name = $1 AND finished_at IS NULL AND rolled_back_at IS NULL"
        );
        client.execute(&statement, &[&name]).await?;
        Ok(())
    }

    async fn roll_back_once(&self, name: &str) -> Result<(), RecoveryError> {
        match timeout(self.command_timeout, self.roll_back_marker(name)).await {
            Err(_)
            | Ok(
                Ok(())
                | Err(
                    RecoveryError::Database { .. }
                    | RecoveryError::Connection {
                        source: ConnectionError::Open { .. },
                    },
                ),
            ) => Ok(()),
            Ok(Err(error)) => Err(error),
        }
    }

    async fn warn_ahead_once(&self) -> Vec<String> {
        let check = MigrationDriftCheck {
            database_url: Some(self.database_url.clone()),
            direct_url: self.direct_url.clone(),
        };
        warn_if_db_ahead_of_head(&check, &self.known).await
    }
}

impl RecoverySteps for PgSteps<'_> {
    fn deploy(&self, timeout: Duration) -> StepFuture<'_, Result<DeployOutcome, RecoveryError>> {
        Box::pin(self.deploy_once(timeout))
    }

    fn recover_completed<'step>(
        &'step self,
        name: &'step str,
    ) -> StepFuture<'step, Result<bool, RecoveryError>> {
        Box::pin(self.recover_once(name))
    }

    fn baseline(&self) -> StepFuture<'_, Result<(), RecoveryError>> {
        Box::pin(self.baseline_once())
    }

    fn failed_history<'step>(
        &'step self,
        identity: &'step FailedIdentity,
    ) -> StepFuture<'step, FailedLookup> {
        Box::pin(look_up_failed_migration(
            Some(&self.prisma_url),
            &identity.name,
            &identity.started_at,
        ))
    }

    fn roll_back<'step>(
        &'step self,
        name: &'step str,
    ) -> StepFuture<'step, Result<(), RecoveryError>> {
        Box::pin(self.roll_back_once(name))
    }

    fn warn_ahead(&self) -> StepFuture<'_, Vec<String>> {
        Box::pin(self.warn_ahead_once())
    }
}

// Mirrors the P3009/P3018 name guard: one path component and no backslash
pub(super) fn recoverable_name(name: &str) -> bool {
    if name.is_empty() || name == "." || name == ".." {
        return false;
    }
    !name.contains('/') && !name.contains('\\')
}
