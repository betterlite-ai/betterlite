use std::str::Utf8Error;

use deadpool_postgres::Client;
use thiserror::Error;
use tokio::time::{Duration, Instant, error::Elapsed, sleep, timeout_at};
use tokio_postgres::{Client as PostgresClient, Error as PostgresError};
use uuid::Uuid;

pub mod catalog;
pub mod fixtures;
mod ledger;
mod tests;

use catalog::MigrationCatalog;

const LOCK_ID: i64 = 72_707_369;

#[derive(Debug, Error)]
pub enum DeployError {
    #[error("database operation failed")]
    Database(#[from] PostgresError),
    #[error("migration {name} has an unfinished ledger row")]
    Unfinished { name: String },
    #[error("migration {name} failed")]
    Migration {
        name: String,
        #[source]
        source: PostgresError,
    },
    #[error("migration {name} is not valid UTF-8")]
    InvalidSql {
        name: String,
        #[source]
        source: Utf8Error,
    },
    #[error("migration deployment deadline elapsed")]
    Deadline(#[source] Elapsed),
}

pub struct MigrationDeploy {
    pub schema: String,
    pub catalog: MigrationCatalog,
    pub next_id: fn() -> Uuid,
}

impl MigrationDeploy {
    pub async fn run(&self, client: Client, deadline: Instant) -> Result<(), DeployError> {
        // A session lock must never return to the pool, including when this future is dropped.
        let connection = Client::take(client);
        let result = timeout_at(deadline, self.apply(&connection, deadline)).await;
        drop(connection);
        result.map_err(DeployError::Deadline)?
    }

    async fn apply(&self, client: &PostgresClient, deadline: Instant) -> Result<(), DeployError> {
        limit(client, deadline).await?;
        loop {
            let acquired: bool = client
                .query_one("SELECT pg_try_advisory_lock($1)", &[&LOCK_ID])
                .await?
                .get(0);
            if acquired {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
        let result = async {
            let schema = ledger::quote(&self.schema);
            client
                .query_one("SELECT set_config('search_path', $1, false)", &[&schema])
                .await?;
            let table = format!("{schema}.\"_prisma_migrations\"");
            let finished = ledger::prepare(client, &table).await?;
            for migration in self
                .catalog
                .iter()
                .filter(|migration| !finished.contains(&migration.name))
            {
                limit(client, deadline).await?;
                ledger::apply(client, &table, migration, (self.next_id)()).await?;
            }
            Ok::<(), DeployError>(())
        }
        .await;
        let unlock = client
            .query_one("SELECT pg_advisory_unlock($1)", &[&LOCK_ID])
            .await
            .map(|_| ());
        match (result, unlock) {
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) => Err(DeployError::Database(error)),
            (Ok(()), Ok(())) => Ok(()),
        }
    }
}

async fn limit(client: &PostgresClient, deadline: Instant) -> Result<(), DeployError> {
    let milliseconds = deadline
        .saturating_duration_since(Instant::now())
        .as_millis()
        .max(1);
    client
        .query_one(
            "SELECT set_config('statement_timeout', $1, false)",
            &[&milliseconds.to_string()],
        )
        .await?;
    Ok(())
}
