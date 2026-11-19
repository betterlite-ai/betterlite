use std::sync::Arc;

use async_trait::async_trait;
use betterlite_db::budget_permission_rows::LiteLLMObjectPermissionTable;
use betterlite_db::database_failures::gate::SystemClock;
use betterlite_db::postgres_store::rows::KeyColumn;
use deadpool_postgres::GenericClient;

use super::managed_rows::{
    ManagedRowsQuery, ManagedVectorStoreRows, VectorStoreRow, VectorStoreRowsFailure,
};
use super::rows::{ManagedVectorStoreTable, ObjectPermissionTable};
use super::vector_store::{ObjectPermissionRows, VectorStoreLookupFailure};
use crate::auth_read::adapter::AuthReadAdapter;
use crate::auth_read::repository::{NativeKey, RepositoryTable};
use crate::auth_read::table::{AuthReadTable, AuthTable, FindMany, FindOne, Presence, ReadContext};

// B725's object permission row and B726's vector_store_id.in read through B2315's bounded tables
#[must_use]
pub fn open_grant_rows<Client, Rows>(
    client: Arc<Client>,
    table: Rows,
    adapter: AuthReadAdapter,
    context: ReadContext<SystemClock>,
) -> AuthReadTable<RepositoryTable<Client, Rows>, SystemClock>
where
    RepositoryTable<Client, Rows>: AuthTable,
{
    adapter.open(
        Arc::new(RepositoryTable {
            client: Some(client),
            table,
        }),
        // both grant adapters name their own lookup, so the caller label is never read
        String::new(),
        context,
    )
}

#[async_trait]
impl<Client> ObjectPermissionRows
    for AuthReadTable<RepositoryTable<Client, ObjectPermissionTable>, SystemClock>
where
    Client: GenericClient + Send + Sync + 'static,
{
    async fn by_id(
        &self,
        object_permission_id: &str,
    ) -> Result<Option<LiteLLMObjectPermissionTable>, VectorStoreLookupFailure> {
        let filter = NativeKey {
            column: KeyColumn::Id,
            value: Box::new(object_permission_id.to_owned()),
        };
        let request = FindOne {
            filter,
            include: Presence::Omitted,
        };
        self.find_unique(request)
            .await
            .map_err(|source| VectorStoreLookupFailure {
                source: Box::new(source),
            })
    }
}

#[async_trait]
impl<Client> ManagedVectorStoreRows
    for AuthReadTable<RepositoryTable<Client, ManagedVectorStoreTable>, SystemClock>
where
    Client: GenericClient + Send + Sync + 'static,
{
    async fn by_uuids(
        &self,
        query: ManagedRowsQuery,
    ) -> Result<Vec<VectorStoreRow>, VectorStoreRowsFailure> {
        let request = FindMany {
            filter: Presence::Value(query.uuids),
            include: Presence::Omitted,
            take: Presence::Value(query.take),
        };
        self.find_many(request)
            .await
            .map_err(|source| VectorStoreRowsFailure {
                source: Box::new(source),
            })
    }
}
