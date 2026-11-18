use std::error::Error;

use deadpool_postgres::{Client as PooledClient, Pool};
use tokio_postgres::types::ToSql;

use crate::database_replica::client::DatabaseClient;
use crate::database_replica::routing::RouteError;
use crate::postgres_store::diagnostics::{DriverError, PoolFailure};
use crate::postgres_store::errors::DbError;

use super::rows::RowError;
use super::sql::Statement;
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
    source: RouteError<ClientError>,
) -> TableOpsError<ClientError>
where
    ClientError: Error + Send + Sync + 'static,
{
    match source {
        RouteError::MissingAttribute { name } => TableOpsError::MissingAttribute { name },
        RouteError::Lookup { source } => TableOpsError::Lookup { source },
    }
}

impl<Client> TableOps<'_, Client>
where
    Client: DatabaseClient,
    Client::Attribute: TablePoolAccess,
{
    pub(super) async fn connection(
        &self,
        attribute: &str,
        method: &str,
    ) -> Result<PooledClient, TableOpsError<Client::Error>> {
        let database = self.database.as_ref().ok_or(TableOpsError::NoDbConnected)?;
        let route = database.attribute(attribute).map_err(routed_name)?;
        let pool = route.select(method).pool();
        pool.get().await.map_err(|driver| TableOpsError::Database {
            source: DbError::Pool {
                source: PoolFailure { driver },
            },
        })
    }

    pub(super) async fn text_rows(
        &self,
        attribute: &str,
        method: &str,
        sql: &str,
        parameters: &[&(dyn ToSql + Sync)],
    ) -> Result<Vec<String>, TableOpsError<Client::Error>> {
        let client = self.connection(attribute, method).await?;
        let rows = client
            .query(sql, parameters)
            .await
            .map_err(|driver| TableOpsError::from(DriverError { driver }))?;
        let mut texts = Vec::with_capacity(rows.len());
        for row in rows {
            texts.push(
                row.try_get(0)
                    .map_err(|driver| TableOpsError::from(DriverError { driver }))?,
            );
        }
        Ok(texts)
    }

    pub(super) async fn text_row(
        &self,
        attribute: &str,
        method: &str,
        sql: &str,
        parameters: &[&(dyn ToSql + Sync)],
    ) -> Result<Option<String>, TableOpsError<Client::Error>> {
        Ok(self
            .text_rows(attribute, method, sql, parameters)
            .await?
            .into_iter()
            .next())
    }

    // every read branch is one retried query plus its own row decoder
    pub(super) async fn query_rows<Row, Parse>(
        &self,
        attribute: &str,
        method: &str,
        sql: &str,
        parameters: &[&(dyn ToSql + Sync)],
        policy: RetryPolicy,
        call_type: OperationCallType,
        parse: Parse,
    ) -> Result<Vec<Row>, TableOpsError<Client::Error>>
    where
        Row: Send,
        Parse: Fn(&str) -> Result<Row, RowError> + Send + Sync,
    {
        self.attempt_with(policy, call_type, || async {
            let rows = self.text_rows(attribute, method, sql, parameters).await?;
            rows.iter()
                .map(|text| parse(text).map_err(TableOpsError::from))
                .collect()
        })
        .await
    }

    pub(super) async fn run_upsert(
        &self,
        attribute: &str,
        statement: &Statement,
    ) -> Result<String, TableOpsError<Client::Error>> {
        match self
            .text_row(
                attribute,
                "upsert",
                &statement.sql,
                &statement.parameters.bind_values(),
            )
            .await?
        {
            Some(text) => Ok(text),
            None => Err(TableOpsError::Database {
                source: DbError::CreateNull,
            }),
        }
    }
}
