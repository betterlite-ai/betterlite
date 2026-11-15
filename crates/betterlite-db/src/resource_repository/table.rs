mod query;
mod sql;

use std::collections::BTreeMap;
use std::marker::PhantomData;

use betterlite_redis::config_sync::publish::{PublishChange, redis_config_change_publisher};
use betterlite_redis::config_sync::table_actions::TableActions;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio_postgres::types::ToSql;

use crate::auxiliary_repository::repository::{TableAccess, TableBinding, TableError};
use crate::budget_permission_rows::JsonData;
use crate::postgres_store::errors::backend;

use self::query::{execute_count, query_all, query_one};
use self::sql::{
    create_statement, document, document_json, generate_id, quote_identifier, update_statement,
    upsert_statement,
};

// The live Prisma tables differ only in their key, generated id and write payloads, so one
// action set serves every resource repository.
pub trait ResourceBinding: TableBinding {
    const KEY: &'static str;
    const ID: Option<&'static str>;
    const UPDATED_AT: Option<&'static str>;
    type Create: Serialize + Send + 'static;
    type Update: Serialize + Send + 'static;
}

// LiteLLM's table actions take a mapping and pass every key through, unknown ones included.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RowWrite(pub BTreeMap<String, JsonData>);

pub struct ResourceActions<'db, B: ResourceBinding> {
    access: TableAccess<'db>,
    binding: PhantomData<B>,
}

impl<'db, B: ResourceBinding> ResourceActions<'db, B> {
    #[must_use]
    pub const fn new(access: TableAccess<'db>) -> Self {
        Self {
            access,
            binding: PhantomData,
        }
    }

    pub async fn find_unique<R>(&self, key: &str) -> Result<Option<R>, TableError>
    where
        R: DeserializeOwned,
    {
        let sql = format!(
            "SELECT row_to_json(t)::text FROM {} AS t WHERE t.{} = $1",
            quote_identifier(B::TABLE),
            quote_identifier(B::KEY)
        );
        query_one(self.access.read, &sql, &[key]).await
    }

    pub async fn find_many<R>(&self) -> Result<Vec<R>, TableError>
    where
        R: DeserializeOwned,
    {
        let sql = format!(
            "SELECT row_to_json(t)::text FROM {} AS t",
            quote_identifier(B::TABLE)
        );
        query_all(self.access.read, &sql, &[]).await
    }

    pub async fn find_excluding<R>(&self, excluded: &str) -> Result<Vec<R>, TableError>
    where
        R: DeserializeOwned,
    {
        let sql = format!(
            "SELECT row_to_json(t)::text FROM {} AS t WHERE t.{} <> $1",
            quote_identifier(B::TABLE),
            quote_identifier(B::KEY)
        );
        query_all(self.access.read, &sql, &[excluded]).await
    }
}

impl<B: ResourceBinding + Send + Sync> TableActions for ResourceActions<'_, B> {
    type Row = JsonData;
    type Filter = String;
    type CreateData = B::Create;
    type UpdateData = B::Update;
    type Failure = TableError;

    async fn create(&self, data: Self::CreateData) -> Result<Self::Row, Self::Failure> {
        let mut record = document(&data)?;
        generate_id::<B>(&mut record);
        let sql = create_statement::<B>(&record);
        let json = document_json(&record)?;
        let params: &[&(dyn ToSql + Sync)] = if record.is_empty() { &[] } else { &[&json] };
        query_one(self.access.write, &sql, params)
            .await?
            .ok_or(TableError::MissingRow)
    }

    async fn create_many(&self, data: Vec<Self::CreateData>) -> Result<usize, Self::Failure> {
        let mut client = self.access.write.session().await?;
        let transaction = client
            .transaction()
            .await
            .map_err(|source| TableError::Database(backend(source)))?;
        for item in &data {
            let mut record = document(item)?;
            generate_id::<B>(&mut record);
            let sql = create_statement::<B>(&record);
            let json = document_json(&record)?;
            let params: &[&(dyn ToSql + Sync)] = if record.is_empty() { &[] } else { &[&json] };
            transaction
                .execute(&sql, params)
                .await
                .map_err(|source| TableError::Database(backend(source)))?;
        }
        transaction
            .commit()
            .await
            .map_err(|source| TableError::Database(backend(source)))?;
        Ok(data.len())
    }

    async fn update(
        &self,
        data: Self::UpdateData,
        filter: Self::Filter,
    ) -> Result<Option<Self::Row>, Self::Failure> {
        let record = document(&data)?;
        let sql = update_statement::<B>(&record);
        let json = document_json(&record)?;
        query_one(self.access.write, &sql, &[&json, &filter]).await
    }

    async fn update_many(
        &self,
        data: Self::UpdateData,
        filter: Self::Filter,
    ) -> Result<usize, Self::Failure> {
        let record = document(&data)?;
        let sql = update_statement::<B>(&record);
        let json = document_json(&record)?;
        execute_count(self.access.write, &sql, &[&json, &filter]).await
    }

    async fn upsert(
        &self,
        filter: Self::Filter,
        data: Self::CreateData,
    ) -> Result<Self::Row, Self::Failure> {
        let mut record = document(&data)?;
        record.insert(B::KEY.to_owned(), JsonData::String(filter));
        generate_id::<B>(&mut record);
        let sql = upsert_statement::<B>(&record);
        let json = document_json(&record)?;
        query_one(self.access.write, &sql, &[&json])
            .await?
            .ok_or(TableError::MissingRow)
    }

    async fn delete(&self, filter: Self::Filter) -> Result<Option<Self::Row>, Self::Failure> {
        let sql = format!(
            "DELETE FROM {} AS t WHERE t.{} = $1 RETURNING row_to_json(t)::text",
            quote_identifier(B::TABLE),
            quote_identifier(B::KEY)
        );
        query_one(self.access.write, &sql, &[&filter]).await
    }

    async fn delete_many(&self, filter: Self::Filter) -> Result<usize, Self::Failure> {
        let sql = format!(
            "DELETE FROM {} AS t WHERE t.{} = $1",
            quote_identifier(B::TABLE),
            quote_identifier(B::KEY)
        );
        execute_count(self.access.write, &sql, &[&filter]).await
    }
}

// Config, budget and permission tables are not config-synced, so their repositories
// publish nothing and never need a Redis handle.
#[must_use]
pub(crate) fn silent_publisher() -> PublishChange<TableError> {
    redis_config_change_publisher(None)
}
