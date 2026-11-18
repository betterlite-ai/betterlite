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

pub type PostgresPermissions<Client> =
    AuthReadTable<RepositoryTable<Client, ObjectPermissionTable>, SystemClock>;

pub type PostgresVectorStores<Client> =
    AuthReadTable<RepositoryTable<Client, ManagedVectorStoreTable>, SystemClock>;

// B725's object permission row, read through B2315's bounded object permission adapter
#[must_use]
pub fn open_permissions<Client: GenericClient + Send + Sync + 'static>(
    client: Arc<Client>,
    context: ReadContext<SystemClock>,
) -> PostgresPermissions<Client> {
    AuthReadAdapter::ObjectPermission.open(
        Arc::new(RepositoryTable {
            client: Some(client),
            table: ObjectPermissionTable,
        }),
        String::from("object_permission"),
        context,
    )
}

// B726's vector_store_id.in query, read through B2315's bounded vector store adapter
#[must_use]
pub fn open_vector_stores<Client: GenericClient + Send + Sync + 'static>(
    client: Arc<Client>,
    context: ReadContext<SystemClock>,
) -> PostgresVectorStores<Client> {
    AuthReadAdapter::VectorStore.open(
        Arc::new(RepositoryTable {
            client: Some(client),
            table: ManagedVectorStoreTable,
        }),
        String::from("vector_store"),
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
            .map_err(|error| VectorStoreLookupFailure {
                message: error.to_string(),
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
            .map_err(|error| VectorStoreRowsFailure {
                message: error.to_string(),
            })
    }
}
