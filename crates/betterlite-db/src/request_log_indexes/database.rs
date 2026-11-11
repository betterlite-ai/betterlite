use std::error::Error as StdError;
use std::future::Future;
use std::pin::Pin;

use deadpool_postgres::Client;

use crate::migration_ledger::lock::MIGRATION_LOCK_KEY;
use crate::request_log_index_catalog::catalog::{
    self, EquivalentIndex, IndexState, Relation, TableKind,
};
use crate::request_log_index_catalog::definitions::RequestLogIndex;

use super::RequestLogIndexError;

pub(crate) type DatabaseFuture<'database, T, E> =
    Pin<Box<dyn Future<Output = Result<T, E>> + Send + 'database>>;

pub(crate) trait DatabaseFailure: StdError + Send + Sync + 'static {
    fn is_database_failure(&self) -> bool;
    fn is_lock_not_available(&self) -> bool;
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
        schema: &'database str,
        table: &'database str,
        expected_name: &'database str,
        index: RequestLogIndex<'database>,
    ) -> DatabaseFuture<'database, Vec<EquivalentIndex>, Self::Error>;

    fn children_without_the_index<'database>(
        &'database self,
        schema: &'database str,
        table: &'database str,
        index: &'database str,
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

impl IndexDatabase for PostgresDatabase {
    type Error = RequestLogIndexError;

    fn table_kind<'database>(
        &'database self,
        schema: &'database str,
        table: &'database str,
    ) -> DatabaseFuture<'database, Option<TableKind>, Self::Error> {
        Box::pin(async move {
            catalog::table_kind(&self.client, schema, table)
                .await
                .map_err(|source| RequestLogIndexError::Catalog { source })
        })
    }

    fn index_state<'database>(
        &'database self,
        schema: &'database str,
        index: &'database str,
    ) -> DatabaseFuture<'database, Option<IndexState>, Self::Error> {
        Box::pin(async move {
            catalog::index_state(&self.client, schema, index)
                .await
                .map_err(|source| RequestLogIndexError::Catalog { source })
        })
    }

    fn equivalent_indexes<'database>(
        &'database self,
        schema: &'database str,
        table: &'database str,
        expected_name: &'database str,
        index: RequestLogIndex<'database>,
    ) -> DatabaseFuture<'database, Vec<EquivalentIndex>, Self::Error> {
        Box::pin(async move {
            catalog::equivalent_indexes(&self.client, schema, table, expected_name, &index)
                .await
                .map_err(|source| RequestLogIndexError::Catalog { source })
        })
    }

    fn children_without_the_index<'database>(
        &'database self,
        schema: &'database str,
        table: &'database str,
        index: &'database str,
    ) -> DatabaseFuture<'database, Vec<Relation>, Self::Error> {
        Box::pin(async move {
            catalog::children_without_the_index(&self.client, schema, table, index)
                .await
                .map_err(|source| RequestLogIndexError::Catalog { source })
        })
    }

    fn execute<'database>(
        &'database self,
        statement: &'database str,
    ) -> DatabaseFuture<'database, (), Self::Error> {
        Box::pin(async move {
            self.client
                .batch_execute(statement)
                .await
                .map_err(|source| RequestLogIndexError::Statement { source })
        })
    }

    fn try_coordinator_lock(&self) -> DatabaseFuture<'_, bool, Self::Error> {
        Box::pin(async move {
            self.client
                .query_opt(
                    "SELECT pg_try_advisory_lock($1) AS acquired",
                    &[&MIGRATION_LOCK_KEY],
                )
                .await
                .map_err(|source| RequestLogIndexError::Statement { source })
                .map(|found| found.is_some_and(|row| row.get("acquired")))
        })
    }

    fn release_coordinator_lock(&self) -> DatabaseFuture<'_, (), Self::Error> {
        Box::pin(async move {
            self.client
                .execute("SELECT pg_advisory_unlock($1)", &[&MIGRATION_LOCK_KEY])
                .await
                .map(|_affected| ())
                .map_err(|source| RequestLogIndexError::Statement { source })
        })
    }
}
