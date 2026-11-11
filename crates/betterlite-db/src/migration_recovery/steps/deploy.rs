use std::time::Duration;

use tokio::time::{Instant, timeout, timeout_at};

use crate::migration_deploy::DeployError;
use crate::migration_ledger::connection::connect;
use crate::migration_ledger::lock::PRISMA_LOCK_KEY;

use super::super::RecoveryError;
use super::super::outcome::{DeployFailure, DeployOutcome, FailedIdentity, FailedStep, StepSource};
use super::PgSteps;
use super::failure::{database_outcome, failed_after_step, step_outcome};

const CONTENTION_DETAIL: &str =
    "the deploy deadline elapsed while another process held the migration lock";

impl PgSteps<'_> {
    // Mirrors Prisma's P3005 check: a namespace with objects but no history refuses to deploy until it is baselined
    async fn needs_baseline(&self, client: &tokio_postgres::Client) -> Result<bool, RecoveryError> {
        let table = format!(
            "\"{}\".\"_prisma_migrations\"",
            self.deploy.schema.replace('"', "\"\"")
        );
        let ledger: Option<String> = client
            .query_one("SELECT to_regclass($1)::text", &[&table])
            .await?
            .try_get(0)?;
        if ledger.is_some() {
            return Ok(false);
        }
        let nonempty: bool = client
            .query_one(
                "SELECT EXISTS (SELECT 1 FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
                 WHERE n.nspname = $1 AND c.relkind IN ('r', 'p', 'v', 'm', 'f'))",
                &[&self.deploy.schema],
            )
            .await?
            .try_get(0)?;
        Ok(nonempty)
    }

    pub(in crate::migration_recovery) async fn unfinished_identity(
        &self,
        name: &str,
    ) -> Result<Option<FailedIdentity>, RecoveryError> {
        let identity = timeout(self.command_timeout, self.identity_row(name)).await;
        identity.unwrap_or_else(|_elapsed| {
            Err(RecoveryError::QueryTimeout {
                seconds: self.command_timeout.as_secs(),
            })
        })
    }

    // The re-read keeps the newest row's identity even after a concurrent replica finished or rolled it back
    async fn identity_row(&self, name: &str) -> Result<Option<FailedIdentity>, RecoveryError> {
        let client = connect(&self.deploy_url).await?;
        let table = format!(
            "\"{}\".\"_prisma_migrations\"",
            self.deploy.schema.replace('"', "\"\"")
        );
        let statement = format!(
            "SELECT started_at::text FROM {table} WHERE migration_name = $1 \
             ORDER BY started_at DESC LIMIT 1"
        );
        let Some(row) = client.query_opt(&statement, &[&name]).await? else {
            return Ok(None);
        };
        let started_at: String = row.try_get(0)?;
        Ok(Some(FailedIdentity {
            name: name.to_owned(),
            started_at,
        }))
    }

    pub(super) async fn deploy_once(
        &self,
        timeout: Duration,
    ) -> Result<DeployOutcome, RecoveryError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|source| RecoveryError::DeployConnection { source })?;
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or(RecoveryError::DeployTimeout)?;
        // The P3005 check runs under the deploy deadline, like the subprocess check inside migrate deploy
        let baseline = match timeout_at(deadline, self.needs_baseline(&client)).await {
            Err(_elapsed) => return Ok(DeployOutcome::TimedOut),
            Ok(result) => result?,
        };
        if baseline {
            return Ok(DeployOutcome::SchemaNotBaselined);
        }
        self.classify(self.deploy.run(client, deadline).await).await
    }

    async fn classify(
        &self,
        result: Result<(), DeployError>,
    ) -> Result<DeployOutcome, RecoveryError> {
        let outcome = match result {
            Ok(()) => DeployOutcome::Complete,
            Err(DeployError::Unfinished { name }) => DeployOutcome::UnfinishedHistory {
                identity: self.unfinished_identity(&name).await?,
            },
            Err(DeployError::Migration { name, source }) => step_outcome(name, source),
            Err(DeployError::InvalidSql { name, source }) => {
                DeployOutcome::FailedStep(FailedStep {
                    name,
                    source: StepSource::Utf8(source),
                })
            }
            Err(DeployError::Database(source)) => database_outcome(source),
            Err(DeployError::MigrationLog {
                name,
                source,
                ledger,
            }) => failed_after_step(name, source, ledger),
            Err(DeployError::InvalidSqlLog {
                name,
                source,
                ledger,
            }) => DeployOutcome::Failed(DeployFailure::InvalidSqlLog {
                name,
                source,
                ledger,
            }),
            Err(DeployError::Deadline(_)) => self.deadline_outcome().await?,
        };
        Ok(outcome)
    }

    async fn deadline_outcome(&self) -> Result<DeployOutcome, RecoveryError> {
        if self.deploy_lock_held().await? {
            return Ok(DeployOutcome::Contention {
                detail: CONTENTION_DETAIL.to_owned(),
            });
        }
        Ok(DeployOutcome::TimedOut)
    }

    // A deadline with a released lock means this deploy held it and ran out of time; a held one means it waited
    pub(in crate::migration_recovery) async fn deploy_lock_held(
        &self,
    ) -> Result<bool, RecoveryError> {
        match timeout(self.command_timeout, self.lock_state()).await {
            Ok(held) => held,
            Err(_elapsed) => Err(RecoveryError::QueryTimeout {
                seconds: self.command_timeout.as_secs(),
            }),
        }
    }

    async fn lock_state(&self) -> Result<bool, RecoveryError> {
        let client = connect(&self.deploy_url).await?;
        let held: bool = client
            .query_one(
                "SELECT EXISTS (SELECT 1 FROM pg_locks WHERE locktype = 'advisory' AND granted \
                 AND objsubid = 1 AND classid::bigint = 0 AND objid::bigint = $1 \
                 AND database = (SELECT oid FROM pg_database WHERE datname = current_database()))",
                &[&PRISMA_LOCK_KEY],
            )
            .await?
            .try_get(0)?;
        Ok(held)
    }
}
