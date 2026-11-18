use std::fmt::Display;

use chrono::{DateTime, Utc};
use deadpool_postgres::Pool;
use tokio_postgres::Row;

use crate::budget_permission_rows::JsonData;
use crate::database_faults::{DbError, ErrorType};
use crate::database_replica::client::{Attribute, DatabaseClient};
use crate::database_replica::pinned::{WriterPinnedClient, writer_wrapper};
use crate::database_replica::routing::{Route, RouteError};
use crate::key_hash::KeyHash;
use crate::key_repository::TokenTableAccess;

use super::grace::{DeprecatedRowError, DeprecatedToken};
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
        Box::pin(async move { combined_row(&self.database, hash, use_writer).await })
    }

    fn deprecated_row<'lookup>(
        &'lookup self,
        hash: &'lookup KeyHash,
        now: DateTime<Utc>,
        use_writer: bool,
    ) -> DeprecatedFuture<'lookup> {
        Box::pin(async move { deprecated_row(&self.database, hash, now, use_writer).await })
    }
}

async fn combined_row<Client>(
    database: &Option<WriterPinnedClient<'_, Client>>,
    hash: &KeyHash,
    use_writer: bool,
) -> Result<Option<JsonData>, DbError>
where
    Client: DatabaseClient,
    Client::Attribute: TokenTableAccess + Send + Sync,
{
    let route = route_for(database, use_writer, QUERY_FIRST_METHOD)?;
    let client = connection(&route, QUERY_FIRST_METHOD).await?;
    let row = client
        .query_opt(COMBINED_KEY_SQL, &[hash])
        .await
        .map_err(query_fault)?;
    row.map(|found| decode_row(&found)).transpose()
}

async fn deprecated_row<Client>(
    database: &Option<WriterPinnedClient<'_, Client>>,
    hash: &KeyHash,
    now: DateTime<Utc>,
    use_writer: bool,
) -> Result<Option<DeprecatedToken>, DbError>
where
    Client: DatabaseClient,
    Client::Attribute: TokenTableAccess + Send + Sync,
{
    let route = route_for(database, use_writer, DEPRECATED_TOKEN_TABLE)?;
    let client = connection(&route, FIND_FIRST_METHOD).await?;
    let naive_now = now.naive_utc();
    let found = client
        .query_opt(DEPRECATED_KEY_SQL, &[hash, &naive_now])
        .await
        .map_err(query_fault)?;
    found
        .map(|row| {
            DeprecatedToken::from_columns(
                row.try_get(0).map_err(record_fault)?,
                row.try_get(1).map_err(record_fault)?,
            )
            .map_err(deprecated_fault)
        })
        .transpose()
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
    pinned.attribute(attribute).map_err(route_fault)
}

// a writer-pinned read goes to the raw writer client, so it never consults the replica routing
fn direct_route<'db, Client>(
    client: &'db Client,
    attribute: &str,
) -> Result<Route<'db, Client::Attribute>, DbError>
where
    Client: DatabaseClient,
{
    match client.attribute(attribute).map_err(route_fault)? {
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

fn decode_row(row: &Row) -> Result<JsonData, DbError> {
    let text: String = row.try_get(0).map_err(record_fault)?;
    serde_json::from_str(&text).map_err(|source| fault(ErrorType::Data, source.to_string()))
}

fn route_fault<Failure>(source: RouteError<Failure>) -> DbError
where
    Failure: Display,
{
    fault(ErrorType::Other, source.to_string())
}

fn deprecated_fault(source: DeprecatedRowError) -> DbError {
    fault(ErrorType::Data, source.to_string())
}

fn query_fault(source: tokio_postgres::Error) -> DbError {
    fault(ErrorType::RawQuery, source.to_string())
}

fn record_fault(source: tokio_postgres::Error) -> DbError {
    fault(ErrorType::Data, source.to_string())
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
