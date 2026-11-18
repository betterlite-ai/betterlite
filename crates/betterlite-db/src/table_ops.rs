pub mod access;
pub mod columns;
mod deletes;
pub mod generic;
mod inserts;
pub mod reads;
pub mod rows;
mod sql;
mod team_json;
pub mod updates;

mod tests;

use std::error::Error;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use tokio::time::timeout;

use crate::database_replica::client::DatabaseClient;
use crate::database_replica::pinned::WriterPinnedClient;
use crate::key_user_writes::retry::WriteClock;
use crate::migration_ledger::lock::JitterSource;
use crate::postgres_store::diagnostics::DriverError;
use crate::postgres_store::errors::DbError;
use crate::tool_spend::RetryPause;

use rows::RowError;

pub const DEFAULT_TEAM_LIST_LIMIT: i64 = 20;
pub const FAILURE_REPORT_DEADLINE: Duration = Duration::from_secs(1);
pub const DEFAULT_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(10);
pub const DEFAULT_QUERY_TIMEOUT: Duration = Duration::from_secs(30);

// LiteLLM's backoff.on_exception wraps every dispatcher with 3 tries in 10 seconds, and get_generic_data with 1 try in 2
const DATA_RETRY: RetryPolicy = RetryPolicy {
    tries: 3,
    budget: Duration::from_secs(10),
};
const GENERIC_RETRY: RetryPolicy = RetryPolicy {
    tries: 1,
    budget: Duration::from_secs(2),
};

#[derive(Debug, Clone, Copy)]
struct RetryPolicy {
    tries: u32,
    budget: Duration,
}

#[derive(Debug, Clone, Copy)]
pub struct TableSettings {
    pub team_list_limit: i64,
    pub acquire_timeout: Duration,
    pub query_timeout: Duration,
}

impl Default for TableSettings {
    fn default() -> Self {
        Self {
            team_list_limit: DEFAULT_TEAM_LIST_LIMIT,
            acquire_timeout: DEFAULT_ACQUIRE_TIMEOUT,
            query_timeout: DEFAULT_QUERY_TIMEOUT,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationCallType {
    GetData,
    GetGenericData,
    InsertData,
    UpdateData,
    DeleteData,
}

impl OperationCallType {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::GetData => "get_data",
            Self::GetGenericData => "get_generic_data",
            Self::InsertData => "insert_data",
            Self::UpdateData => "update_data",
            Self::DeleteData => "delete_data",
        }
    }
}

pub type FailureFuture<'sink> = Pin<Box<dyn Future<Output = ()> + Send + 'sink>>;

pub struct TableFailureEvent<'error, ClientError>
where
    ClientError: Error + Send + Sync + 'static,
{
    pub call_type: OperationCallType,
    pub error: &'error TableOpsError<ClientError>,
    pub traceback: String,
    pub duration: Duration,
}

pub trait FailureSink<ClientError>: Send + Sync
where
    ClientError: Error + Send + Sync + 'static,
{
    fn record(&self, event: TableFailureEvent<'_, ClientError>) -> FailureFuture<'_>;
}

pub type InvalidateFuture<'name> = Pin<Box<dyn Future<Output = ()> + Send + 'name>>;

// LiteLLM awaits invalidate_config_param after a config write, which evicts the cache and publishes the change
pub trait ConfigParamInvalidator: Send + Sync {
    fn invalidate_param<'call>(&'call self, param_name: &'call str) -> InvalidateFuture<'call>;
}

pub struct TablePorts<'ports, ClientError>
where
    ClientError: Error + Send + Sync + 'static,
{
    pub clock: &'ports dyn WriteClock,
    pub jitter: &'ports dyn JitterSource,
    pub pause: &'ports dyn RetryPause,
    pub sink: &'ports dyn FailureSink<ClientError>,
    pub invalidator: &'ports dyn ConfigParamInvalidator,
}

pub struct TableOps<'service, Client>
where
    Client: DatabaseClient,
{
    pub database: Option<WriterPinnedClient<'service, Client>>,
    pub ports: TablePorts<'service, Client::Error>,
    pub settings: TableSettings,
}

#[derive(Debug, thiserror::Error)]
pub enum TableOpsError<ClientError>
where
    ClientError: Error + Send + Sync + 'static,
{
    #[error("No DB Connected. See - https://docs.litellm.ai/docs/proxy/virtual_keys")]
    NoDbConnected,
    #[error("the database operation failed")]
    Database {
        #[source]
        source: DbError<RowError>,
    },
    #[error("the database has no {name} table accessor")]
    MissingAttribute { name: String },
    #[error("the database attribute lookup failed")]
    Lookup {
        #[source]
        source: ClientError,
    },
    #[error("the write payload needs the {field} value")]
    MissingField { field: &'static str },
    #[error("the write value for {column} does not fit its column type")]
    WrongType { column: &'static str },
    #[error("the write value for {column} cannot be encoded")]
    Encode {
        column: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("the {operation} did not finish within {deadline:?}")]
    Timeout {
        operation: &'static str,
        deadline: Duration,
    },
}

impl<ClientError> TableOpsError<ClientError>
where
    ClientError: Error + Send + Sync + 'static,
{
    #[must_use]
    pub const fn database_source(&self) -> Option<&DbError<RowError>> {
        match self {
            Self::Database { source } => Some(source),
            Self::NoDbConnected
            | Self::MissingAttribute { .. }
            | Self::Lookup { .. }
            | Self::MissingField { .. }
            | Self::WrongType { .. }
            | Self::Encode { .. }
            | Self::Timeout { .. } => None,
        }
    }
}

impl<ClientError> From<DbError<RowError>> for TableOpsError<ClientError>
where
    ClientError: Error + Send + Sync + 'static,
{
    fn from(source: DbError<RowError>) -> Self {
        Self::Database { source }
    }
}

impl<ClientError> From<RowError> for TableOpsError<ClientError>
where
    ClientError: Error + Send + Sync + 'static,
{
    fn from(source: RowError) -> Self {
        Self::Database {
            source: DbError::InvalidRecord { source },
        }
    }
}

impl<ClientError> From<DriverError> for TableOpsError<ClientError>
where
    ClientError: Error + Send + Sync + 'static,
{
    fn from(driver: DriverError) -> Self {
        Self::Database {
            source: DbError::Backend { source: driver },
        }
    }
}

impl<Client> TableOps<'_, Client>
where
    Client: DatabaseClient,
{
    // the source retries every failure, so a decode error runs the query again too
    async fn attempt_with<Output, Attempt, AttemptFuture>(
        &self,
        policy: RetryPolicy,
        call_type: OperationCallType,
        attempt: Attempt,
    ) -> Result<Output, TableOpsError<Client::Error>>
    where
        Output: Send,
        Attempt: Fn() -> AttemptFuture + Send,
        AttemptFuture: Future<Output = Result<Output, TableOpsError<Client::Error>>> + Send,
    {
        let started = self.ports.clock.instant();
        let mut tries: u32 = 0;
        loop {
            tries = tries.saturating_add(1);
            let attempt_started = self.ports.clock.instant();
            let outcome = attempt().await;
            let duration = self
                .ports
                .clock
                .instant()
                .saturating_duration_since(attempt_started);
            let error = match outcome {
                Ok(output) => return Ok(output),
                Err(error) => error,
            };
            self.report(call_type, &error, duration).await;
            let elapsed = self
                .ports
                .clock
                .instant()
                .saturating_duration_since(started);
            // the counter stops at the policy's try count, so equality is the last-try test
            if tries == policy.tries || elapsed >= policy.budget {
                return Err(error);
            }
            self.ports
                .pause
                .pause(self.backoff(tries, elapsed, policy.budget))
                .await;
        }
    }

    // backoff's expo waiter doubles after each failed try, jittered across the whole range and capped by the budget
    fn backoff(&self, tries: u32, elapsed: Duration, budget: Duration) -> Duration {
        let exponent = tries.saturating_sub(1).min(31);
        let seconds = 1_u32.checked_shl(exponent).unwrap_or(u32::MAX);
        let span = Duration::from_secs(u64::from(seconds));
        let nanos = u64::try_from(span.as_nanos()).unwrap_or(u64::MAX);
        let jittered = Duration::from_nanos(self.ports.jitter.uniform_nanos(nanos));
        jittered.min(budget.saturating_sub(elapsed))
    }

    async fn report(
        &self,
        call_type: OperationCallType,
        error: &TableOpsError<Client::Error>,
        duration: Duration,
    ) {
        let heading = match call_type {
            OperationCallType::GetData => "LiteLLM Prisma Client Exception: Error with `get_data`",
            OperationCallType::GetGenericData => "LiteLLM Prisma Client Exception get_generic_data",
            OperationCallType::InsertData => "LiteLLM Prisma Client Exception in insert_data",
            OperationCallType::UpdateData => "LiteLLM Prisma Client Exception - update_data",
            OperationCallType::DeleteData => "LiteLLM Prisma Client Exception - delete_data",
        };
        let mut traceback = format!("{heading}: {error}");
        let mut cause = error.source();
        while let Some(source) = cause {
            traceback.push_str("\nCaused by: ");
            traceback.push_str(&source.to_string());
            cause = source.source();
        }
        let first_line = traceback.lines().next().unwrap_or_default().to_owned();
        tracing::debug!("{first_line}");
        let record = self.ports.sink.record(TableFailureEvent {
            call_type,
            error,
            traceback,
            duration,
        });
        if timeout(FAILURE_REPORT_DEADLINE, record).await.is_err() {
            tracing::warn!("the failure sink did not report the failed table operation in time");
        }
    }
}
