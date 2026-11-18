mod deadline;

use std::error::Error;
use std::future::Future;
use std::time::Duration;

use deadpool_postgres::{Client as PooledClient, Pool};
use tokio::time::timeout;
use tokio_postgres::types::ToSql;

use crate::database_replica::client::DatabaseClient;
use crate::database_replica::routing::RouteError;
use crate::postgres_store::diagnostics::{DriverError, PoolFailure};
use crate::postgres_store::errors::DbError;

use super::rows::RowError;
use super::sql::{Bound, Statement};
use super::{OperationCallType, RetryPolicy, TableOps, TableOpsError};

pub const USER_ATTRIBUTE: &str = "litellm_usertable";
pub const KEY_ATTRIBUTE: &str = "litellm_verificationtoken";
pub const CONFIG_ATTRIBUTE: &str = "litellm_config";
pub const SPEND_ATTRIBUTE: &str = "litellm_spendlogs";
pub const BUDGET_ATTRIBUTE: &str = "litellm_budgettable";
pub const END_USER_ATTRIBUTE: &str = "litellm_endusertable";
pub const TEAM_ATTRIBUTE: &str = "litellm_teamtable";
pub const NOTIFICATION_ATTRIBUTE: &str = "litellm_usernotifications";
pub const RAW_QUERY_ATTRIBUTE: &str = "query_raw";

pub const USER_TABLE: &str = "LiteLLM_UserTable";
pub const KEY_TABLE: &str = "LiteLLM_VerificationToken";
pub const CONFIG_TABLE: &str = "LiteLLM_Config";
pub const SPEND_TABLE: &str = "LiteLLM_SpendLogs";
pub const BUDGET_TABLE: &str = "LiteLLM_BudgetTable";
pub const END_USER_TABLE: &str = "LiteLLM_EndUserTable";
pub const TEAM_TABLE: &str = "LiteLLM_TeamTable";
pub const NOTIFICATION_TABLE: &str = "LiteLLM_UserNotifications";

pub trait TablePoolAccess: Send + Sync {
    fn pool(&self) -> &Pool;
}

impl TablePoolAccess for Pool {
    fn pool(&self) -> &Pool {
        self
    }
}

// Prisma's find_* methods read from the replica and every write goes to the writer, as B714 routes them
pub(super) fn routed_name<ClientError>(
    route_error: RouteError<ClientError>,
) -> TableOpsError<ClientError>
where
    ClientError: Error + Send + Sync + 'static,
{
    match route_error {
        RouteError::MissingAttribute { name } => TableOpsError::MissingAttribute { name },
        RouteError::Lookup { source } => TableOpsError::Lookup { source },
    }
}

pub(super) struct ReadQuery<'query> {
    pub(super) attribute: &'query str,
    pub(super) method: &'query str,
    pub(super) sql: &'query str,
    pub(super) parameters: &'query [&'query (dyn ToSql + Sync)],
}

pub(super) trait TableAccess<Client>
where
    Client: DatabaseClient,
    Client::Attribute: TablePoolAccess,
{
    fn connection(
        &self,
        attribute: &str,
        method: &str,
    ) -> impl Future<Output = Result<PooledClient, TableOpsError<Client::Error>>> + Send;
    fn text_rows(
        &self,
        attribute: &str,
        method: &str,
        sql: &str,
        parameters: &[&(dyn ToSql + Sync)],
    ) -> impl Future<Output = Result<Vec<String>, TableOpsError<Client::Error>>> + Send;
    fn query_rows<Row, Parse>(
        &self,
        query: ReadQuery<'_>,
        policy: RetryPolicy,
        call_type: OperationCallType,
        parse: Parse,
    ) -> impl Future<Output = Result<Vec<Row>, TableOpsError<Client::Error>>> + Send
    where
        Row: Send,
        Parse: Fn(&str) -> Result<Row, RowError> + Send + Sync;
    fn run_upsert(
        &self,
        attribute: &str,
        statement: &Statement,
    ) -> impl Future<Output = Result<String, TableOpsError<Client::Error>>> + Send;
    fn execute_statements(
        &self,
        attribute: &str,
        statements: &[Statement],
    ) -> impl Future<Output = Result<(), TableOpsError<Client::Error>>> + Send;
}

impl<Client> TableAccess<Client> for TableOps<'_, Client>
where
    Client: DatabaseClient,
    Client::Attribute: TablePoolAccess,
{
    async fn connection(
        &self,
        attribute: &str,
        method: &str,
    ) -> Result<PooledClient, TableOpsError<Client::Error>> {
        let database = self.database.as_ref().ok_or(TableOpsError::NoDbConnected)?;
        let route = database.attribute(attribute).map_err(routed_name)?;
        let pool = route.select(method).pool();
        let deadline = self.settings.acquire_timeout;
        bounded("acquire", deadline, pool.get())
            .await?
            .map_err(|driver| TableOpsError::Database {
                source: DbError::Pool {
                    source: PoolFailure { driver },
                },
            })
    }

    async fn text_rows(
        &self,
        attribute: &str,
        method: &str,
        sql: &str,
        parameters: &[&(dyn ToSql + Sync)],
    ) -> Result<Vec<String>, TableOpsError<Client::Error>> {
        let mut client = self.connection(attribute, method).await?;
        let outcome =
            deadline::query(&mut client, sql, parameters, self.settings.query_timeout).await;
        if matches!(outcome, Err(TableOpsError::Timeout { .. })) {
            drop(PooledClient::take(client));
        }
        let rows = outcome?;
        let mut texts = Vec::with_capacity(rows.len());
        for row in rows {
            texts.push(
                row.try_get(0)
                    .map_err(|driver| TableOpsError::from(DriverError { driver }))?,
            );
        }
        Ok(texts)
    }

    // every read branch is one retried query plus its own row decoder
    async fn query_rows<Row, Parse>(
        &self,
        query: ReadQuery<'_>,
        policy: RetryPolicy,
        call_type: OperationCallType,
        parse: Parse,
    ) -> Result<Vec<Row>, TableOpsError<Client::Error>>
    where
        Row: Send,
        Parse: Fn(&str) -> Result<Row, RowError> + Send + Sync,
    {
        self.attempt_with(policy, call_type, || async {
            let rows = self
                .text_rows(query.attribute, query.method, query.sql, query.parameters)
                .await?;
            rows.iter()
                .map(|text| parse(text).map_err(TableOpsError::from))
                .collect()
        })
        .await
    }

    // a write returns the stored row, so one that matched no row has nothing to return
    async fn run_upsert(
        &self,
        attribute: &str,
        statement: &Statement,
    ) -> Result<String, TableOpsError<Client::Error>> {
        let parameters: Vec<&(dyn ToSql + Sync)> = statement
            .parameters
            .values
            .iter()
            .map(Bound::as_sql)
            .collect();
        let rows = self
            .text_rows(attribute, "upsert", &statement.sql, &parameters)
            .await?;
        rows.into_iter().next().ok_or(TableOpsError::Database {
            source: DbError::CreateNull,
        })
    }

    // LiteLLM writes a whole batch through one transaction, so a failure loses every row in it
    async fn execute_statements(
        &self,
        attribute: &str,
        statements: &[Statement],
    ) -> Result<(), TableOpsError<Client::Error>> {
        let mut client = self.connection(attribute, "update_many").await?;
        let outcome = run_batch(&mut client, statements, self.settings.query_timeout).await;
        if matches!(outcome, Err(TableOpsError::Timeout { .. })) {
            drop(PooledClient::take(client));
        }
        outcome
    }
}

// every database wait gets a finite deadline, so a drained pool or a lock wait fails the call instead of hanging
async fn bounded<ClientError, CallFuture, Output>(
    operation: &'static str,
    deadline: Duration,
    call: CallFuture,
) -> Result<Output, TableOpsError<ClientError>>
where
    CallFuture: Future<Output = Output>,
    ClientError: Error + Send + Sync + 'static,
{
    timeout(deadline, call)
        .await
        .map_err(|_elapsed| TableOpsError::Timeout {
            operation,
            deadline,
        })
}

async fn run_batch<ClientError>(
    client: &mut PooledClient,
    statements: &[Statement],
    deadline: Duration,
) -> Result<(), TableOpsError<ClientError>>
where
    ClientError: Error + Send + Sync + 'static,
{
    let transaction = deadline::begin(client, deadline).await?;
    for statement in statements {
        let parameters: Vec<&(dyn ToSql + Sync)> = statement
            .parameters
            .values
            .iter()
            .map(Bound::as_sql)
            .collect();
        deadline::statement(
            "statement",
            deadline,
            transaction.execute(&statement.sql, &parameters),
        )
        .await?;
    }
    deadline::statement("commit", deadline, transaction.commit()).await?;
    Ok(())
}
