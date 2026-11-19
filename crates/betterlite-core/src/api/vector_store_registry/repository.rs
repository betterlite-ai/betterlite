use std::future::Future;
use std::sync::Arc;

use betterlite_db::auxiliary_repository::bindings::{
    ManagedVectorStoreIndexTable as IndexBinding, ManagedVectorStoresTable as StoresBinding,
};
use betterlite_db::auxiliary_repository::repository::{
    PrismaClient, PrismaTableRepository, TableError,
};
use betterlite_db::postgres_store::errors::backend;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_openai::api::vector_store_records::records::pydantic::LiteLlmManagedVectorStoreIndex;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;
use betterlite_redis::config_sync::publish::PublishChange;
use deadpool_postgres::GenericClient as _;
use deadpool_postgres::tokio_postgres::Row;
use serde::de::DeserializeOwned;

// find_many(order={"created_at": "desc"}) shapes both tables the same way
const STORES_SQL: &str = "SELECT to_jsonb(row)::text FROM \"LiteLLM_ManagedVectorStoresTable\" AS row ORDER BY row.created_at DESC";

const INDEXES_SQL: &str = "SELECT to_jsonb(row)::text FROM \"LiteLLM_ManagedVectorStoreIndexTable\" AS row ORDER BY row.created_at DESC";

// the two table reads the registry makes, so a test answers them without Postgres
pub trait VectorStoreDatabase: Send + Sync {
    fn stores(
        &self,
    ) -> impl Future<Output = Result<Vec<LiteLlmManagedVectorStore>, TableError>> + Send;

    fn indexes(
        &self,
    ) -> impl Future<Output = Result<Vec<LiteLlmManagedVectorStoreIndex>, TableError>> + Send;
}

// _get_vector_stores_from_db: a missing client loads nothing rather than reaching the database
pub async fn stores_from_database<D>(
    database: Option<&D>,
) -> Result<Vec<LiteLlmManagedVectorStore>, TableError>
where
    D: VectorStoreDatabase,
{
    match database {
        Some(source) => source.stores().await,
        None => Ok(Vec::new()),
    }
}

// _get_vector_store_indexes_from_db: the same missing-client answer for the index table
pub async fn indexes_from_database<D: VectorStoreDatabase>(
    database: Option<&D>,
) -> Result<Vec<LiteLlmManagedVectorStoreIndex>, TableError> {
    let Some(client) = database else {
        return Ok(Vec::new());
    };
    client.indexes().await
}

fn decode_row<T>(row: &Row) -> Result<T, TableError>
where
    T: DeserializeOwned,
{
    let text: String = row
        .try_get(0)
        .map_err(|source| TableError::Database(backend(source)))?;
    serde_json::from_str(&text).map_err(TableError::Row)
}

async fn query_rows<T>(store: &PostgresStore, sql: &str) -> Result<Vec<T>, TableError>
where
    T: DeserializeOwned,
{
    let client = store.session().await?;
    let rows = client
        .query(sql, &[])
        .await
        .map_err(|source| TableError::Database(backend(source)))?;
    rows.iter().map(decode_row).collect()
}

pub struct PrismaVectorStoreDatabase<'db> {
    stores: PrismaTableRepository<'db, StoresBinding>,
    indexes: PrismaTableRepository<'db, IndexBinding>,
}

impl<'db> PrismaVectorStoreDatabase<'db> {
    #[must_use]
    pub fn new(client: Option<PrismaClient<'db>>, publish: PublishChange<TableError>) -> Self {
        Self {
            stores: PrismaTableRepository::new(client, Arc::clone(&publish), StoresBinding),
            indexes: PrismaTableRepository::new(client, publish, IndexBinding),
        }
    }
}

impl VectorStoreDatabase for PrismaVectorStoreDatabase<'_> {
    async fn stores(&self) -> Result<Vec<LiteLlmManagedVectorStore>, TableError> {
        let access = self.stores.access()?;
        query_rows(access.read, STORES_SQL).await
    }

    async fn indexes(&self) -> Result<Vec<LiteLlmManagedVectorStoreIndex>, TableError> {
        let access = self.indexes.access()?;
        query_rows(access.read, INDEXES_SQL).await
    }
}
