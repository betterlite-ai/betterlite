use std::error::Error as StdError;
use std::future::Future;
use std::pin::Pin;

use deadpool_postgres::Client;

use crate::migration_ledger::lock::{HeldLockError, held_migration_lock};
use crate::request_log_index_catalog::catalog::{
    self, EquivalentIndex, IndexState, Relation, RequestLogIndexCatalogError, TableKind,
};
use crate::request_log_index_catalog::definitions::RequestLogIndex;

use super::RequestLogIndexError;

pub(crate) type DatabaseFuture<'database, T, E> =
    Pin<Box<dyn Future<Output = Result<T, E>> + Send + 'database>>;

pub(crate) trait DatabaseFailure: StdError + Send + Sync + 'static {
    fn is_database_failure(&self) -> bool;
    fn is_lock_not_available(&self) -> bool;
}

#[derive(Clone, Copy)]
pub(crate) struct IndexRef<'names> {
    pub(crate) schema: &'names str,
    pub(crate) table: &'names str,
    pub(crate) name: &'names str,
}

pub(crate) trait IndexDatabase: Send + Sync {
    type Error: DatabaseFailure;

    fn table_kind<'database>(
        &'database self,
        schema: &'database str,
        table: &'database str,
    ) -> DatabaseFuture<'database, Option<TableKind>, Self::Error>;

    fn index_state<'database>(
        &'database self,
        schema: &'database str,
        index: &'database str,
    ) -> DatabaseFuture<'database, Option<IndexState>, Self::Error>;

    fn equivalent_indexes<'database>(
        &'database self,
        target: IndexRef<'database>,
        definition: RequestLogIndex<'database>,
    ) -> DatabaseFuture<'database, Vec<EquivalentIndex>, Self::Error>;

    fn children_without_the_index<'database>(
        &'database self,
        target: IndexRef<'database>,
    ) -> DatabaseFuture<'database, Vec<Relation>, Self::Error>;

    fn execute<'database>(
        &'database self,
        statement: &'database str,
    ) -> DatabaseFuture<'database, (), Self::Error>;

    fn coordinate<'database>(
        &'database self,
        step: DatabaseFuture<'database, bool, Self::Error>,
    ) -> DatabaseFuture<'database, bool, Self::Error>;
}

pub(crate) struct PostgresDatabase {
    pub(crate) client: Client,
}

fn catalog_future<'database, T, Q>(query: Q) -> DatabaseFuture<'database, T, RequestLogIndexError>
where
    T: 'database,
    Q: Future<Output = Result<T, RequestLogIndexCatalogError>> + Send + 'database,
{
    Box::pin(async move {
        let outcome = query.await;
        outcome.map_err(RequestLogIndexError::from)
    })
}

async fn equivalent_query(
    client: &Client,
    target: IndexRef<'_>,
    definition: RequestLogIndex<'_>,
) -> Result<Vec<EquivalentIndex>, RequestLogIndexCatalogError> {
    catalog::equivalent_indexes(
        client,
        target.schema,
        target.table,
        target.name,
        &definition,
    )
    .await
}

impl IndexDatabase for PostgresDatabase {
    type Error = RequestLogIndexError;

    fn table_kind<'database>(
        &'database self,
        schema: &'database str,
        table: &'database str,
    ) -> DatabaseFuture<'database, Option<TableKind>, Self::Error> {
        Box::pin(async move { Ok(catalog::table_kind(&self.client, schema, table).await?) })
    }

    fn index_state<'database>(
        &'database self,
        schema: &'database str,
        index: &'database str,
    ) -> DatabaseFuture<'database, Option<IndexState>, Self::Error> {
        Box::pin(async move { Ok(catalog::index_state(&self.client, schema, index).await?) })
    }

    fn equivalent_indexes<'database>(
        &'database self,
        target: IndexRef<'database>,
        definition: RequestLogIndex<'database>,
    ) -> DatabaseFuture<'database, Vec<EquivalentIndex>, Self::Error> {
        catalog_future(equivalent_query(&self.client, target, definition))
    }

    fn children_without_the_index<'database>(
        &'database self,
        target: IndexRef<'database>,
    ) -> DatabaseFuture<'database, Vec<Relation>, Self::Error> {
        Box::pin(async move {
            let children = catalog::children_without_the_index(
                &self.client,
                target.schema,
                target.table,
                target.name,
            );
            Ok(children.await?)
        })
    }

    fn execute<'database>(
        &'database self,
        statement: &'database str,
    ) -> DatabaseFuture<'database, (), Self::Error> {
        Box::pin(execute_statement(&self.client, statement))
    }

    fn coordinate<'database>(
        &'database self,
        step: DatabaseFuture<'database, bool, Self::Error>,
    ) -> DatabaseFuture<'database, bool, Self::Error> {
        Box::pin(async move {
            let held = held_migration_lock(&self.client, move |acquired| {
                coordinated_step(step, acquired)
            })
            .await;
            match held {
                Ok(value) => Ok(value),
                Err(HeldLockError::Body(error)) => Err(error),
                Err(HeldLockError::Acquire { source } | HeldLockError::Unlock { source }) => {
                    Err(RequestLogIndexError::from(source))
                }
            }
        })
    }
}

async fn coordinated_step(
    step: DatabaseFuture<'_, bool, RequestLogIndexError>,
    acquired: bool,
) -> Result<bool, RequestLogIndexError> {
    if !acquired {
        tracing::info!(
            "Another process holds the migration lock, leaving the request-log indexes to the next index build"
        );
        return Ok(false);
    }
    step.await
}

async fn execute_statement(client: &Client, statement: &str) -> Result<(), RequestLogIndexError> {
    client
        .batch_execute(statement)
        .await
        .map_err(RequestLogIndexError::from)
}
