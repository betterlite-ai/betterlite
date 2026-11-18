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

use super::grace::DeprecatedToken;
use super::ports::{DeprecatedFuture, KeyReadDatabase, QUERY_FIRST_METHOD, RowFuture};
use super::sql::{COMBINED_KEY_SQL, DEPRECATED_KEY_SQL};

pub const DEPRECATED_TOKEN_TABLE: &str = "litellm_deprecatedverificationtoken";
const FIND_FIRST_METHOD: &str = "find_first";

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
    ) -> Result<Option<JsonData>, DbError> {
        let found = self
            .fetch_row(
                use_writer,
                QUERY_FIRST_METHOD,
                QUERY_FIRST_METHOD,
                COMBINED_KEY_SQL,
                &[hash],
            )
            .await?;
        let Some(row) = found else {
            return Ok(None);
        };
        let text: String = row
            .try_get(0)
            .map_err(|source| fault(ErrorType::Data, source.to_string()))?;
        let fields = serde_json::from_str(&text)
            .map_err(|source| fault(ErrorType::Data, source.to_string()))?;
        Ok(Some(fields))
    }

    async fn deprecated_token(
        &self,
        hash: &KeyHash,
        now: DateTime<Utc>,
        use_writer: bool,
    ) -> Result<Option<DeprecatedToken>, DbError> {
        let naive_now = now.naive_utc();
        let found = self
            .fetch_row(
                use_writer,
                DEPRECATED_TOKEN_TABLE,
                FIND_FIRST_METHOD,
                DEPRECATED_KEY_SQL,
                &[hash, &naive_now],
            )
            .await?;
        found
            .map(|row| {
                let active_token_id = row
                    .try_get(0)
                    .map_err(|source| fault(ErrorType::Data, source.to_string()))?;
                let revoke_at = row
                    .try_get(1)
                    .map_err(|source| fault(ErrorType::Data, source.to_string()))?;
                DeprecatedToken::from_columns(active_token_id, revoke_at)
                    .map_err(|source| fault(ErrorType::Data, source.to_string()))
            })
            .transpose()
    }

    async fn fetch_row(
        &self,
        use_writer: bool,
        attribute: &str,
        method: &str,
        sql: &str,
        parameters: &[&(dyn ToSql + Sync)],
    ) -> Result<Option<Row>, DbError> {
        let route = route_for(&self.database, use_writer, attribute)?;
        let client = connection(&route, method).await?;
        client
            .query_opt(sql, parameters)
            .await
            .map_err(|source| fault(ErrorType::RawQuery, source.to_string()))
    }
}

fn route_for<'db, Client>(
    database: &'db Option<WriterPinnedClient<'db, Client>>,
    use_writer: bool,
    attribute: &str,
) -> Result<Route<'db, Client::Attribute>, DbError>
where
    Client: DatabaseClient,
{
    let Some(pinned) = database.as_ref() else {
        return Err(no_database());
    };
    if use_writer {
        return direct_route(writer_wrapper(*pinned), attribute);
    }
    pinned
        .attribute(attribute)
        .map_err(|source| fault(ErrorType::Other, source.to_string()))
}

// a writer-pinned read goes to the raw writer client, so it never consults the replica routing
fn direct_route<'db, Client>(
    client: &'db Client,
    attribute: &str,
) -> Result<Route<'db, Client::Attribute>, DbError>
where
    Client: DatabaseClient,
{
    let attribute_result = client
        .attribute(attribute)
        .map_err(|source| fault(ErrorType::Other, source.to_string()))?;
    match attribute_result {
        Some(Attribute::Direct(accessor) | Attribute::Model(accessor)) => {
            Ok(Route::Direct(accessor))
        }
        None => Err(fault(
            ErrorType::Other,
            format!("the database has no attribute named {attribute}"),
        )),
    }
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
        fault(
            ErrorType::Connect,
            format!("database pool unavailable: {source}"),
        )
    })
}

fn no_database() -> DbError {
    fault(
        ErrorType::NoDbConnection,
        String::from("No DB Connected. See - https://docs.litellm.ai/docs/proxy/virtual_keys"),
    )
}

fn fault(error_type: ErrorType, message: String) -> DbError {
    DbError {
        error_type,
        message,
        code: None,
        meta_code: None,
        traceback_modules: Vec::new(),
        cause: None,
    }
}
