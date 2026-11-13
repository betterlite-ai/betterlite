use std::future::Future;
use std::pin::Pin;

use deadpool_postgres::GenericClient;
use thiserror::Error;
use tokio_postgres::types::Type;

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
        Box::pin(async move {
            let (values, types): (Vec<_>, Vec<_>) = args.iter().map(SqlBind::parameter).unzip();
            // the repository decodes stored records from JSON text, so every query returns its jsonb as text
            let statement = self
                .writer
                .prepare_typed(query, &types)
                .await
                .map_err(DatabaseError::Prepare)?;
            let rows = self
                .writer
                .query(&statement, &values)
                .await
                .map_err(DatabaseError::Query)?;
            rows.iter()
                .map(|row| row.try_get::<_, String>("data").map(|data| Row { data }))
                .collect::<Result<Vec<_>, _>>()
                .map_err(DatabaseError::Row)
        })
    }

    fn execute_raw<'database>(
        &'database self,
        query: &'database str,
        args: &'database [SqlBind],
    ) -> ExecuteFuture<'database> {
        Box::pin(async move {
            let (values, types): (Vec<_>, Vec<_>) = args.iter().map(SqlBind::parameter).unzip();
            let statement = self
                .writer
                .prepare_typed(query, &types)
                .await
                .map_err(DatabaseError::Prepare)?;
            self.writer
                .execute(&statement, &values)
                .await
                .map_err(DatabaseError::Query)
        })
    }
}

mod tests;
