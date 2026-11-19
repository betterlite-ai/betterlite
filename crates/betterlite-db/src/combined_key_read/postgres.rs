use std::sync::Arc;

use chrono::{DateTime, Utc};
use deadpool_postgres::Pool;
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;

use crate::budget_permission_rows::JsonData;
use crate::database_faults::{DbError, ErrorType};
use crate::database_replica::client::{Attribute, DatabaseClient};
use crate::database_replica::pinned::{WriterPinnedClient, writer_wrapper};
use crate::database_replica::routing::Route;
use crate::key_hash::KeyHash;
use crate::key_repository::TokenTableAccess;
use crate::postgres_store::diagnostics::DriverError;

use super::error::{DatabaseFailure, database_fault};
use super::grace::DeprecatedToken;
use super::ports::{DeprecatedFuture, KeyReadDatabase, QUERY_FIRST_METHOD, RowFuture};
use super::sql::{COMBINED_KEY_SQL, DEPRECATED_KEY_SQL};

pub const DEPRECATED_TOKEN_TABLE: &str = "litellm_deprecatedverificationtoken";
const FIND_FIRST_METHOD: &str = "find_first";

// the client attribute a query resolves through and the method name its pool selection reads
#[derive(Clone, Copy)]
struct TableSpec {
    attribute: &'static str,
    method: &'static str,
}

pub struct PostgresKeyDatabase<'db, Client> {
    pub database: Option<WriterPinnedClient<'db, Client>>,
}

impl<Client> KeyReadDatabase for PostgresKeyDatabase<'_, Client>
where
    Client: DatabaseClient,
    Client::Attribute: TokenTableAccess + Send + Sync,
{
    fn read_row<'lookup>(
        &'lookup self,
        hash: &'lookup KeyHash,
        use_writer: bool,
    ) -> RowFuture<'lookup> {
        Box::pin(async move { self.combined_row(hash, use_writer).await })
    }

    fn deprecated_row<'lookup>(
        &'lookup self,
        hash: &'lookup KeyHash,
        now: DateTime<Utc>,
        use_writer: bool,
    ) -> DeprecatedFuture<'lookup> {
        Box::pin(async move { self.deprecated_token(hash, now, use_writer).await })
    }
}

impl<Client> PostgresKeyDatabase<'_, Client>
where
    Client: DatabaseClient,
    Client::Attribute: TokenTableAccess + Send + Sync,
{
    async fn combined_row(
        &self,
        hash: &KeyHash,
        use_writer: bool,
    ) -> Result<Option<JsonData>, DatabaseFailure> {
        let table = TableSpec {
            attribute: QUERY_FIRST_METHOD,
            method: QUERY_FIRST_METHOD,
        };
        let found = self
            .fetch_row(use_writer, table, COMBINED_KEY_SQL, &[hash])
            .await?;
        let Some(row) = found else {
            return Ok(None);
        };
        let text: String = row
            .try_get(0)
            .map_err(|source| database_fault(ErrorType::Data, source.to_string()))?;
        let fields = serde_json::from_str(&text)
            .map_err(|source| database_fault(ErrorType::Data, source.to_string()))?;
        Ok(Some(fields))
    }

    async fn deprecated_token(
        &self,
        hash: &KeyHash,
        now: DateTime<Utc>,
        use_writer: bool,
    ) -> Result<Option<DeprecatedToken>, DatabaseFailure> {
        let naive_now = now.naive_utc();
        let table = TableSpec {
            attribute: DEPRECATED_TOKEN_TABLE,
            method: FIND_FIRST_METHOD,
        };
        let found = self
            .fetch_row(use_writer, table, DEPRECATED_KEY_SQL, &[hash, &naive_now])
            .await?;
        found
            .map(|row| {
                let active_token_id = row
                    .try_get(0)
                    .map_err(|source| database_fault(ErrorType::Data, source.to_string()))?;
                let revoke_at = row
                    .try_get(1)
                    .map_err(|source| database_fault(ErrorType::Data, source.to_string()))?;
                DeprecatedToken::from_columns(active_token_id, revoke_at)
                    .map_err(|source| database_fault(ErrorType::Data, source.to_string()))
            })
            .transpose()
            .map_err(DatabaseFailure::from)
    }

    async fn fetch_row(
        &self,
        use_writer: bool,
        table: TableSpec,
        sql: &str,
        parameters: &[&(dyn ToSql + Sync)],
    ) -> Result<Option<Row>, DatabaseFailure> {
        let Some(pinned) = self.database.as_ref() else {
            return Err(DatabaseFailure::from(database_fault(
                ErrorType::NoDbConnection,
                String::from(
                    "No DB Connected. See - https://docs.litellm.ai/docs/proxy/virtual_keys",
                ),
            )));
        };
        // a writer-pinned read goes to the raw writer client, so it never consults the replica routing
        let route = if use_writer {
            let found = writer_wrapper(*pinned)
                .attribute(table.attribute)
                .map_err(|source| database_fault(ErrorType::Other, source.to_string()))?;
            match found {
                Some(Attribute::Direct(accessor) | Attribute::Model(accessor)) => {
                    Route::Direct(accessor)
                }
                None => return Err(missing_attribute(table.attribute)),
            }
        } else {
            pinned
                .attribute(table.attribute)
                .map_err(|source| database_fault(ErrorType::Other, source.to_string()))?
        };
        connection(&route, table.method)
            .await?
            .query_opt(sql, parameters)
            .await
            .map_err(|source| DatabaseFailure::Driver(Arc::new(DriverError { driver: source })))
    }
}

fn missing_attribute(attribute: &str) -> DatabaseFailure {
    DatabaseFailure::from(database_fault(
        ErrorType::Other,
        format!("the database has no attribute named {attribute}"),
    ))
}

async fn connection<Accessor>(
    route: &Route<'_, Accessor>,
    method: &str,
) -> Result<deadpool_postgres::Client, DbError>
where
    Accessor: TokenTableAccess,
{
    let pool: &Pool = route.select(method).pool();
    pool.get().await.map_err(|source| {
        database_fault(
            ErrorType::Connect,
            format!("database pool unavailable: {source}"),
        )
    })
}
