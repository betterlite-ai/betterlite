use std::time::Duration;

use crate::migration_deploy::DeployError;
use crate::migration_ledger::connection::connect;
use crate::migration_ledger::lock::PRISMA_LOCK_KEY;
use tokio::time::Instant;

use super::super::RecoveryError;
use super::super::outcome::{DeployOutcome, FailedIdentity, FailedStep};
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

    async fn unfinished_identity(
        &self,
        name: &str,
    ) -> Result<Option<FailedIdentity>, RecoveryError> {
        let client = connect(&self.deploy_url).await?;
        let table = format!(
            "\"{}\".\"_prisma_migrations\"",
            self.deploy.schema.replace('"', "\"\"")
        );
        let statement = format!(
            "SELECT started_at::text FROM {table} WHERE migration_name = $1 \
             AND finished_at IS NULL AND rolled_back_at IS NULL ORDER BY started_at LIMIT 1"
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
        if self.needs_baseline(&client).await? {
            return Ok(DeployOutcome::SchemaNotBaselined);
        }
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or(RecoveryError::DeployTimeout)?;
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
            Err(DeployError::Migration { name, source }) => step_outcome(name, &source),
            Err(DeployError::InvalidSql { name, source }) => {
                DeployOutcome::FailedStep(FailedStep {
                    name: Some(name),
                    sqlstate: None,
                    detail: source.to_string(),
                })
            }
            Err(DeployError::Database(source)) => database_outcome(&source),
            Err(DeployError::MigrationLog {
                name,
                source,
                ledger,
            }) => failed_after_step(&name, &source, &ledger),
            Err(DeployError::InvalidSqlLog {
                name,
                source,
                ledger,
            }) => DeployOutcome::Failed {
                detail: format!(
                    "migration {name} has SQL that is not UTF-8 ({source}) and recording the failure also failed: {ledger}"
                ),
            },
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
    async fn deploy_lock_held(&self) -> Result<bool, RecoveryError> {
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
