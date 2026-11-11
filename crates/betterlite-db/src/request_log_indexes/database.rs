use std::error::Error as StdError;
use std::future::Future;
use std::pin::Pin;

use deadpool_postgres::Client;

use crate::migration_ledger::lock::MIGRATION_LOCK_KEY;
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

    fn try_coordinator_lock(&self) -> DatabaseFuture<'_, bool, Self::Error>;

    fn release_coordinator_lock(&self) -> DatabaseFuture<'_, (), Self::Error>;
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

    fn try_coordinator_lock(&self) -> DatabaseFuture<'_, bool, Self::Error> {
        Box::pin(probe_coordinator(&self.client))
    }

    fn release_coordinator_lock(&self) -> DatabaseFuture<'_, (), Self::Error> {
        Box::pin(unlock_coordinator(&self.client))
    }
}

async fn execute_statement(client: &Client, statement: &str) -> Result<(), RequestLogIndexError> {
    client
        .batch_execute(statement)
        .await
        .map_err(RequestLogIndexError::from)
}

async fn probe_coordinator(client: &Client) -> Result<bool, RequestLogIndexError> {
    client
        .query_opt(
            "SELECT pg_try_advisory_lock($1) AS acquired",
            &[&MIGRATION_LOCK_KEY],
        )
        .await
        .map_err(RequestLogIndexError::from)
        .map(|found| found.is_some_and(|row| row.get("acquired")))
}

async fn unlock_coordinator(client: &Client) -> Result<(), RequestLogIndexError> {
    client
        .execute("SELECT pg_advisory_unlock($1)", &[&MIGRATION_LOCK_KEY])
        .await
        .map(|_affected| ())
        .map_err(RequestLogIndexError::from)
}
