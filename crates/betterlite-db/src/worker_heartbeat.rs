use std::error::Error as StdError;
use std::fmt::Display;
use std::future::Future;
use std::pin::Pin;

use tokio_postgres::types::ToSql;
use uuid::Uuid;

use crate::database_replica::pinned::{WriterPinnedClient, writer_wrapper};

mod tests;

// LiteLLM fixes the heartbeat interval and three times it for the liveness window
pub const HEARTBEAT_INTERVAL_SECONDS: u32 = 60;
pub const LIVENESS_WINDOW_SECONDS: u32 = HEARTBEAT_INTERVAL_SECONDS.saturating_mul(3);
pub const STALE_ROW_RETENTION_SECONDS: u32 = 3600;

const BEAT_SQL: &str = r#"INSERT INTO "LiteLLM_ProxyWorkerHeartbeat" (worker_id, hostname, last_heartbeat_at)
VALUES ($1, $2, NOW())
ON CONFLICT (worker_id) DO UPDATE SET last_heartbeat_at = NOW()"#;

const PRUNE_SQL: &str = r#"DELETE FROM "LiteLLM_ProxyWorkerHeartbeat"
WHERE last_heartbeat_at < NOW() - make_interval(secs => $1)"#;

const COUNT_SQL: &str = r#"SELECT COUNT(*)::int AS live_workers FROM "LiteLLM_ProxyWorkerHeartbeat"
WHERE last_heartbeat_at > NOW() - make_interval(secs => $1)"#;

const DEREGISTER_SQL: &str = r#"DELETE FROM "LiteLLM_ProxyWorkerHeartbeat" WHERE worker_id = $1"#;

pub type HeartbeatFuture<'database, Value, Failure> =
    Pin<Box<dyn Future<Output = Result<Value, Failure>> + Send + 'database>>;

// the service holds no clock: every timestamp comes from Postgres NOW()
pub trait HeartbeatDatabase: Sync {
    type Error: StdError + Send + Sync + 'static;

    fn upsert<'database>(
        &'database self,
        worker_id: &'database str,
        hostname: &'database str,
    ) -> HeartbeatFuture<'database, (), Self::Error>;

    fn prune(&self, retention_seconds: f64) -> HeartbeatFuture<'_, (), Self::Error>;

    fn deregister<'database>(
        &'database self,
        worker_id: &'database str,
    ) -> HeartbeatFuture<'database, (), Self::Error>;

    fn count_live_workers(
        &self,
        window_seconds: f64,
    ) -> HeartbeatFuture<'_, Vec<Option<i32>>, Self::Error>;
}

pub struct PostgresDatabase<Client> {
    pub client: Client,
}

impl<Client> PostgresDatabase<Client>
where
    Client: deadpool_postgres::GenericClient + Send + Sync,
{
    fn execute_statement<'database, const PARAMETER_COUNT: usize>(
        &'database self,
        sql: &'static str,
        parameters: [&'database (dyn ToSql + Sync); PARAMETER_COUNT],
    ) -> HeartbeatFuture<'database, (), tokio_postgres::Error> {
        Box::pin(async move {
            self.client.execute_raw(sql, parameters).await?;
            Ok(())
        })
    }
}

impl<Client> HeartbeatDatabase for PostgresDatabase<Client>
where
    Client: deadpool_postgres::GenericClient + Send + Sync,
{
    type Error = tokio_postgres::Error;

    fn upsert<'database>(
        &'database self,
        worker_id: &'database str,
        hostname: &'database str,
    ) -> HeartbeatFuture<'database, (), Self::Error> {
        Box::pin(async move {
            self.execute_statement(BEAT_SQL, [&worker_id, &hostname])
                .await
        })
    }

    fn prune(&self, retention_seconds: f64) -> HeartbeatFuture<'_, (), Self::Error> {
        Box::pin(async move {
            self.execute_statement(PRUNE_SQL, [&retention_seconds])
                .await
        })
    }

    fn deregister<'database>(
        &'database self,
        worker_id: &'database str,
    ) -> HeartbeatFuture<'database, (), Self::Error> {
        Box::pin(async move { self.execute_statement(DEREGISTER_SQL, [&worker_id]).await })
    }

    fn count_live_workers(
        &self,
        window_seconds: f64,
    ) -> HeartbeatFuture<'_, Vec<Option<i32>>, Self::Error> {
        Box::pin(async move {
            let rows = self.client.query(COUNT_SQL, &[&window_seconds]).await?;
            Ok(rows
                .iter()
                .map(|row| row.try_get::<_, i32>("live_workers").ok())
                .collect())
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum HostnameError {
    #[error("the operating system hostname could not be read")]
    System {
        #[source]
        source: whoami::Error,
    },
    #[error("the hostname provider failed")]
    Provider,
}

pub trait WorkerIdentity {
    fn hostname(&self) -> Result<String, HostnameError>;
    fn generate_worker_id(&self) -> String;
}

pub struct SystemWorkerIdentity;

impl WorkerIdentity for SystemWorkerIdentity {
    fn hostname(&self) -> Result<String, HostnameError> {
        whoami::hostname().map_err(|source| HostnameError::System { source })
    }

    fn generate_worker_id(&self) -> String {
        Uuid::new_v4().to_string()
    }
}

pub struct ProxyWorkerHeartbeat {
    worker_id: String,
    hostname: String,
}

impl ProxyWorkerHeartbeat {
    pub fn new(
        worker_id: Option<&str>,
        identity: &dyn WorkerIdentity,
    ) -> Result<Self, HostnameError> {
        let hostname = identity.hostname()?;
        let resolved = worker_id
            .filter(|text| !text.is_empty())
            .map_or_else(|| identity.generate_worker_id(), str::to_owned);
        Ok(Self {
            worker_id: resolved,
            hostname,
        })
    }

    pub async fn beat<Database>(&self, database: &Database)
    where
        Database: HeartbeatDatabase,
    {
        let retention = f64::from(STALE_ROW_RETENTION_SECONDS);
        let outcome = database.upsert(&self.worker_id, &self.hostname).await;
        report_failure(&outcome, "Proxy worker heartbeat write failed");
        if outcome.is_ok() {
            let pruned = database.prune(retention).await;
            report_failure(&pruned, "Proxy worker heartbeat write failed");
        }
    }

    pub async fn deregister<Database>(&self, database: &Database)
    where
        Database: HeartbeatDatabase,
    {
        let outcome = database.deregister(&self.worker_id).await;
        report_failure(&outcome, "Proxy worker heartbeat deregister failed");
    }
}

// every heartbeat statement is best effort: its error is logged at DEBUG and never propagated
fn report_failure<Value, Failure>(outcome: &Result<Value, Failure>, message: &str)
where
    Failure: Display,
{
    if let Err(error) = outcome {
        tracing::debug!("{message}: {error}");
    }
}

#[derive(Debug, thiserror::Error)]
enum CensusError<DatabaseError>
where
    DatabaseError: StdError + Send + Sync + 'static,
{
    #[error("the database did not answer")]
    Database {
        #[source]
        source: DatabaseError,
    },
    #[error("the result has no live worker count row")]
    MissingRow,
    #[error("a live worker count row is not an integer")]
    InvalidRow,
}

pub async fn count_live_proxy_workers<Database>(
    database: WriterPinnedClient<'_, Database>,
) -> Option<i32>
where
    Database: HeartbeatDatabase,
{
    let writer = writer_wrapper(database);
    let outcome = census(writer).await;
    report_failure(&outcome, "Live proxy worker count unavailable");
    outcome.ok()
}

async fn census<Database>(database: &Database) -> Result<i32, CensusError<Database::Error>>
where
    Database: HeartbeatDatabase,
{
    let counts = database
        .count_live_workers(f64::from(LIVENESS_WINDOW_SECONDS))
        .await
        .map_err(|source| CensusError::Database { source })?;
    first_valid_count(&counts)
}

fn first_valid_count<DatabaseError>(
    counts: &[Option<i32>],
) -> Result<i32, CensusError<DatabaseError>>
where
    DatabaseError: StdError + Send + Sync + 'static,
{
    let mut first = None;
    for row in counts {
        let count = (*row).ok_or(CensusError::InvalidRow)?;
        first.get_or_insert(count);
    }
    first.ok_or(CensusError::MissingRow)
}
