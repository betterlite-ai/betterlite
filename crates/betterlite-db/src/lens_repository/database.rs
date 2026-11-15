use std::future::Future;
use std::pin::Pin;

use deadpool_postgres::GenericClient;
use thiserror::Error;
use tokio_postgres::Statement;
use tokio_postgres::types::ToSql;

use crate::daily_spend_sql::bind::SqlBind;

pub struct Row {
    pub data: String,
}

pub type QueryFuture<'database> =
    Pin<Box<dyn Future<Output = Result<Vec<Row>, DatabaseError>> + Send + 'database>>;

pub type ExecuteFuture<'database> =
    Pin<Box<dyn Future<Output = Result<u64, DatabaseError>> + Send + 'database>>;

#[derive(Debug, Error)]
pub enum DatabaseError {
    #[error("the database could not prepare the statement")]
    Prepare(#[source] tokio_postgres::Error),
    #[error("the database call failed")]
    Query(#[source] tokio_postgres::Error),
    #[error("a result row carries no text data column")]
    Row(#[source] tokio_postgres::Error),
}

pub trait Database: Send + Sync {
    fn query_raw<'database>(
        &'database self,
        query: &'database str,
        args: &'database [SqlBind],
    ) -> QueryFuture<'database>;

    fn execute_raw<'database>(
        &'database self,
        query: &'database str,
        args: &'database [SqlBind],
    ) -> ExecuteFuture<'database>;
}

pub struct WriterDatabase<Client> {
    pub writer: Client,
}

impl<Client> Database for WriterDatabase<Client>
where
    Client: GenericClient + Send + Sync,
{
    fn query_raw<'database>(
        &'database self,
        query: &'database str,
        args: &'database [SqlBind],
    ) -> QueryFuture<'database> {
        query_rows(&self.writer, query, args)
    }

    fn execute_raw<'database>(
        &'database self,
        query: &'database str,
        args: &'database [SqlBind],
    ) -> ExecuteFuture<'database> {
        Box::pin(execute_prepared(&self.writer, query, args))
    }
}

async fn prepared<'arguments, Client>(
    writer: &Client,
    query: &str,
    args: &'arguments [SqlBind],
) -> Result<(Statement, Vec<&'arguments (dyn ToSql + Sync)>), DatabaseError>
where
    Client: GenericClient,
{
    let (values, types): (Vec<_>, Vec<_>) = args.iter().map(SqlBind::parameter).unzip();
    let statement = writer
        .prepare_typed(query, &types)
        .await
        .map_err(DatabaseError::Prepare)?;
    Ok((statement, values))
}

fn decode_rows(rows: &[tokio_postgres::Row]) -> Result<Vec<Row>, DatabaseError> {
    rows.iter()
        .map(|row| row.try_get::<_, String>("data").map(|data| Row { data }))
        .collect::<Result<Vec<_>, _>>()
        .map_err(DatabaseError::Row)
}

// the repository decodes stored records from JSON text, so every query returns its jsonb as text
fn query_rows<'database, Client>(
    writer: &'database Client,
    query: &'database str,
    args: &'database [SqlBind],
) -> QueryFuture<'database>
where
    Client: GenericClient + Send + Sync,
{
    Box::pin(async move {
        let (statement, values) = prepared(writer, query, args).await?;
        let rows = writer
            .query(&statement, &values)
            .await
            .map_err(DatabaseError::Query)?;
        decode_rows(&rows)
    })
}

async fn execute_prepared<Client>(
    writer: &Client,
    query: &str,
    args: &[SqlBind],
) -> Result<u64, DatabaseError>
where
    Client: GenericClient,
{
    let (statement, values) = prepared(writer, query, args).await?;
    writer
        .execute(&statement, &values)
        .await
        .map_err(DatabaseError::Query)
}

mod tests;
